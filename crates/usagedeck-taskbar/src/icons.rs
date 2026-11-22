//! Embedded provider brand icons and app icon decoded via Win32 WIC into
//! premultiplied 32bpp BGRA buffers for Direct2D rendering.
//!
//! Zero external image-decoding crates: uses `IWICImagingFactory` once at startup
//! to select the highest-resolution frame from each `.ico`/`.png`, scales cleanly
//! to 64×64 with Fant filtering, bakes an antialiased rounded-corner alpha mask,
//! and pre-computes an inverted Codex variant for light surfaces.

use usagedeck_core::view::ProviderKind;
use windows::core::{IUnknown, Result};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DCRenderTarget, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapFrameDecode,
    IWICImagingFactory, IWICPalette, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

const CLAUDE_ICON_BYTES: &[u8] = include_bytes!("../assets/claude.ico");
const CURSOR_ICON_BYTES: &[u8] = include_bytes!("../assets/cursor.ico");
const GROK_ICON_BYTES: &[u8] = include_bytes!("../assets/grokbot.ico");
const OPENAI_ICON_BYTES: &[u8] = include_bytes!("../assets/openai.png");
const OPENCODE_ICON_BYTES: &[u8] = include_bytes!("../assets/opencode.ico");
const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

const ICON_PX: u32 = 64;
/// Rounded-corner radius on 64×64 matching `3px` on a `16×16` icon.
const CORNER_RADIUS_PX: f32 = 12.0;

struct DecodedIcon {
    width: u32,
    height: u32,
    pbgra: Vec<u8>,
}

/// Pre-decoded premultiplied-alpha icon atlas shared by the taskbar strip,
/// flyout cards, and native Settings dialog.
pub struct IconAtlas {
    claude: Option<DecodedIcon>,
    cursor: Option<DecodedIcon>,
    codex_dark: Option<DecodedIcon>,
    codex_light: Option<DecodedIcon>,
    grok: Option<DecodedIcon>,
    opencode: Option<DecodedIcon>,
    app: Option<DecodedIcon>,
}

impl Default for IconAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl IconAtlas {
    /// Decode all embedded brand icons and the app icon via WIC.
    ///
    /// Infallible: any asset that fails to decode (or if COM/WIC is unavailable)
    /// is stored as `None` so callers cleanly fall back to vector glyphs.
    pub fn new() -> Self {
        let no_outer: Option<&IUnknown> = None;
        // SAFETY: in-process WIC imaging factory creation on a COM-initialized thread.
        let factory: Result<IWICImagingFactory> =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, no_outer, CLSCTX_INPROC_SERVER) };
        let Ok(factory) = factory else {
            log::warn!("WIC imaging factory unavailable; falling back to glyph badges");
            return Self {
                claude: None,
                cursor: None,
                codex_dark: None,
                codex_light: None,
                grok: None,
                opencode: None,
                app: None,
            };
        };

        let load = |name: &str, bytes: &'static [u8]| -> Option<DecodedIcon> {
            match decode_icon(&factory, bytes) {
                Ok(icon) => Some(icon),
                Err(e) => {
                    log::warn!("failed to decode embedded icon {name}: {e}");
                    None
                }
            }
        };

        let claude = load("claude.ico", CLAUDE_ICON_BYTES);
        let cursor = load("cursor.ico", CURSOR_ICON_BYTES);
        let codex_dark = load("openai.png", OPENAI_ICON_BYTES);
        let codex_light = codex_dark.as_ref().map(invert_premultiplied_icon);
        let grok = load("grokbot.ico", GROK_ICON_BYTES);
        let opencode = load("opencode.ico", OPENCODE_ICON_BYTES);
        let app = load("icon.ico", APP_ICON_BYTES);

        Self {
            claude,
            cursor,
            codex_dark,
            codex_light,
            grok,
            opencode,
            app,
        }
    }

    /// Draw a provider's brand icon into `rect`. Returns `false` if unavailable
    /// so the caller can render its fallback badge glyph.
    pub fn draw_provider(
        &self,
        rt: &ID2D1DCRenderTarget,
        kind: ProviderKind,
        rect: D2D_RECT_F,
        light_surface: bool,
    ) -> bool {
        let icon = match kind {
            ProviderKind::Claude => self.claude.as_ref(),
            ProviderKind::Cursor => self.cursor.as_ref(),
            ProviderKind::Codex => {
                if light_surface {
                    self.codex_light.as_ref()
                } else {
                    self.codex_dark.as_ref()
                }
            }
            ProviderKind::Grok => self.grok.as_ref(),
            ProviderKind::OpenCode => self.opencode.as_ref(),
        };
        match icon {
            Some(decoded) => draw_decoded(rt, decoded, rect),
            None => false,
        }
    }

    /// Draw the UsageDeck application icon into `rect`. Returns `false` if unavailable.
    #[allow(dead_code)]
    pub fn draw_app_icon(&self, rt: &ID2D1DCRenderTarget, rect: D2D_RECT_F) -> bool {
        match self.app.as_ref() {
            Some(decoded) => draw_decoded(rt, decoded, rect),
            None => false,
        }
    }
}

fn draw_decoded(rt: &ID2D1DCRenderTarget, icon: &DecodedIcon, rect: D2D_RECT_F) -> bool {
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let size = D2D_SIZE_U {
        width: icon.width,
        height: icon.height,
    };
    // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair and `icon.pbgra` holds
    // `width * height * 4` bytes of premultiplied 32bpp BGRA pixels.
    unsafe {
        let Ok(bmp) = rt.CreateBitmap(
            size,
            Some(icon.pbgra.as_ptr().cast()),
            icon.width * 4,
            &props,
        ) else {
            return false;
        };
        rt.DrawBitmap(
            &bmp,
            Some(&rect),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            None,
        );
    }
    true
}

fn decode_icon(factory: &IWICImagingFactory, bytes: &'static [u8]) -> Result<DecodedIcon> {
    let no_palette: Option<&IWICPalette> = None;

    // SAFETY: `bytes` is `'static`, outliving the WIC stream and decoder; all COM
    // interfaces are dropped before returning the owned pixel buffer.
    let mut pbgra = unsafe {
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = factory.CreateDecoderFromStream(
            &stream,
            std::ptr::null(),
            WICDecodeMetadataCacheOnDemand,
        )?;

        let frame_count = decoder.GetFrameCount()?;
        let mut best_frame: Option<IWICBitmapFrameDecode> = None;
        let mut best_area = 0u64;
        for idx in 0..frame_count {
            let Ok(frame) = decoder.GetFrame(idx) else {
                continue;
            };
            let (mut w, mut h) = (0u32, 0u32);
            if frame.GetSize(&mut w, &mut h).is_err() {
                continue;
            }
            let area = u64::from(w) * u64::from(h);
            if best_frame.is_none() || area > best_area {
                best_area = area;
                best_frame = Some(frame);
            }
        }
        let frame = match best_frame {
            Some(f) => f,
            None => decoder.GetFrame(0)?,
        };

        // Convert to premultiplied 32bpp BGRA before scaling so indexed/paletted ICO frames
        // (such as 4bpp grokbot.ico) scale cleanly with Fant filtering and no dark halos.
        let pre_conv = factory.CreateFormatConverter()?;
        pre_conv.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            no_palette,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&pre_conv, ICON_PX, ICON_PX, WICBitmapInterpolationModeFant)?;

        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            no_palette,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stride = ICON_PX * 4;
        let mut buf = vec![0u8; (stride * ICON_PX) as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut buf)?;
        buf
    };

    apply_rounded_corner_mask(&mut pbgra, ICON_PX, ICON_PX, CORNER_RADIUS_PX);
    Ok(DecodedIcon {
        width: ICON_PX,
        height: ICON_PX,
        pbgra,
    })
}

/// Bake an antialiased rounded-rectangle alpha mask into premultiplied 32bpp BGRA pixels.
fn apply_rounded_corner_mask(pbgra: &mut [u8], width: u32, height: u32, radius: f32) {
    let hx = width as f32 * 0.5;
    let hy = height as f32 * 0.5;
    let r = radius.clamp(0.0, hx.min(hy));
    let bx = hx - r;
    let by = hy - r;

    for y in 0..height {
        let dy = (y as f32 + 0.5 - hy).abs();
        let qy = (dy - by).max(0.0);
        if qy <= 0.0 {
            continue;
        }
        for x in 0..width {
            let dx = (x as f32 + 0.5 - hx).abs();
            let qx = (dx - bx).max(0.0);
            if qx <= 0.0 {
                continue;
            }
            let dist = qx.hypot(qy) - r;
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov >= 1.0 {
                continue;
            }
            let off = ((y * width + x) * 4) as usize;
            let Some(px) = pbgra.get_mut(off..off + 4) else {
                continue;
            };
            if cov <= 0.0 {
                px.copy_from_slice(&[0, 0, 0, 0]);
            } else {
                let a = (f32::from(px[3]) * cov).round().clamp(0.0, 255.0) as u8;
                px[3] = a;
                for ch in &mut px[0..3] {
                    *ch = ((f32::from(*ch) * cov).round().clamp(0.0, 255.0) as u8).min(a);
                }
            }
        }
    }
}

/// Invert RGB channels in premultiplied 32bpp BGRA space (`c' = a - c`) so a white
/// glyph on transparent becomes a crisp dark glyph for light surfaces.
fn invert_premultiplied_icon(src: &DecodedIcon) -> DecodedIcon {
    let mut pbgra = src.pbgra.clone();
    invert_premultiplied_bgra(&mut pbgra);
    DecodedIcon {
        width: src.width,
        height: src.height,
        pbgra,
    }
}

fn invert_premultiplied_bgra(pbgra: &mut [u8]) {
    for px in pbgra.as_chunks_mut::<4>().0 {
        let a = px[3];
        px[0] = a.saturating_sub(px[0]);
        px[1] = a.saturating_sub(px[1]);
        px[2] = a.saturating_sub(px[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_corner_mask_clears_corners_and_preserves_center() {
        let mut buf = vec![255u8; (ICON_PX * ICON_PX * 4) as usize];
        apply_rounded_corner_mask(&mut buf, ICON_PX, ICON_PX, CORNER_RADIUS_PX);

        // Top-left corner pixel (0, 0) is strictly outside the r=12 corner arc.
        assert_eq!(&buf[0..4], &[0, 0, 0, 0]);
        // Center pixel (32, 32) is untouched.
        let center = ((32 * ICON_PX + 32) * 4) as usize;
        assert_eq!(&buf[center..center + 4], &[255, 255, 255, 255]);
        // Premultiplied invariant `c <= a` holds everywhere.
        for px in buf.as_chunks::<4>().0 {
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3]);
        }
    }

    #[test]
    fn invert_premultiplied_bgra_flips_white_to_black_preserving_alpha() {
        let mut buf = [
            255, 255, 255, 255, // opaque white -> opaque black
            128, 128, 128, 128, // 50% white -> 50% black
            0, 0, 0, 0, // transparent -> transparent
            0, 0, 0, 200, // black @ 200 -> white @ 200
        ];
        invert_premultiplied_bgra(&mut buf);
        assert_eq!(
            buf,
            [0, 0, 0, 255, 0, 0, 0, 128, 0, 0, 0, 0, 200, 200, 200, 200]
        );
    }
}
