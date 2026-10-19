//! Theme sampler: average brightness of the screen just around the widget (port of
//! desktop/main.js:231-303, `sampleBackground` / `startBgSampler`).
//!
//! Every 3 s, while `themeMode` is "auto" and the widget is visible, GDI captures only the
//! widget's rect plus a 48 px margin (clamped to the widget's monitor), skips the widget's own
//! pixels and emits the mean luminance 0..1 to the widget as `widget-bg`. Otherwise it sleeps 5 s.

use crate::store::ThemeMode;
use crate::{Core, WIDGET};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

pub const EVENT_BG: &str = "widget-bg";
const MARGIN: i32 = 48;
/// Sample every 2nd pixel in each direction (the old code sampled a ~320 px wide thumbnail).
const STRIDE: usize = 2;
const ACTIVE_DELAY: Duration = Duration::from_secs(3);
const IDLE_DELAY: Duration = Duration::from_secs(5);

/// A screen rectangle in physical pixels, `x1`/`y1` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    fn clamp_to(self, b: Rect) -> Rect {
        Rect {
            x0: self.x0.clamp(b.x0, b.x1),
            y0: self.y0.clamp(b.y0, b.y1),
            x1: self.x1.clamp(b.x0, b.x1),
            y1: self.y1.clamp(b.y0, b.y1),
        }
    }
    fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }
    fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// Mean luminance (0..1) of a top-down BGRA buffer covering `area`, skipping pixels inside
/// `skip`. Same weights as the old sampler: 0.0722 B + 0.7152 G + 0.2126 R.
pub fn average_luminance(bgra: &[u8], area: Rect, skip: Rect, stride: usize) -> Option<f64> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if bgra.len() < w * h * 4 {
        return None;
    }
    let mut sum = 0.0;
    let mut count = 0u64;
    for row in (0..h).step_by(stride.max(1)) {
        for col in (0..w).step_by(stride.max(1)) {
            if skip.contains(area.x0 + col as i32, area.y0 + row as i32) {
                continue;
            }
            let i = (row * w + col) * 4;
            sum += (0.0722 * bgra[i] as f64
                + 0.7152 * bgra[i + 1] as f64
                + 0.2126 * bgra[i + 2] as f64)
                / 255.0;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f64)
}

/// Copy `area` of the screen into a top-down BGRA buffer with GDI.
fn capture(area: Rect) -> Result<Vec<u8>, String> {
    let (w, h) = (area.width(), area.height());
    if w == 0 || h == 0 {
        return Err("empty capture area".into());
    }
    // SAFETY: plain GDI calls; every handle created here is released before returning.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let blit = BitBlt(mem, 0, 0, w, h, Some(screen), area.x0, area.y0, SRCCOPY);
        SelectObject(mem, old);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = if blit.is_ok() {
            GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blit.map_err(|e| format!("BitBlt failed: {e}"))?;
        if lines != h {
            return Err(format!("GetDIBits copied {lines} of {h} lines"));
        }
        Ok(buf)
    }
}

/// Average brightness around the widget, on the monitor the widget is on.
fn sample(widget: &WebviewWindow) -> Result<Option<f64>, String> {
    let pos = widget.outer_position().map_err(|e| e.to_string())?;
    let size = widget.outer_size().map_err(|e| e.to_string())?;
    let monitor = widget
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("widget is on no monitor")?;
    let (mp, ms) = (monitor.position(), monitor.size());
    let display = Rect {
        x0: mp.x,
        y0: mp.y,
        x1: mp.x + ms.width as i32,
        y1: mp.y + ms.height as i32,
    };
    let inner = Rect {
        x0: pos.x,
        y0: pos.y,
        x1: pos.x + size.width as i32,
        y1: pos.y + size.height as i32,
    };
    let outer = Rect {
        x0: inner.x0 - MARGIN,
        y0: inner.y0 - MARGIN,
        x1: inner.x1 + MARGIN,
        y1: inner.y1 + MARGIN,
    }
    .clamp_to(display);
    let pixels = capture(outer)?;
    Ok(average_luminance(&pixels, outer, inner, STRIDE))
}

static FAILED_ONCE: AtomicBool = AtomicBool::new(false);
static EMITTED: AtomicU64 = AtomicU64::new(0);

/// One sampler step; returns how long to wait before the next.
fn tick(app: &AppHandle) -> Duration {
    let auto = app
        .try_state::<Core>()
        .is_some_and(|core| core.store.get_settings().theme_mode == ThemeMode::Auto);
    let Some(widget) = app.get_webview_window(WIDGET) else {
        return IDLE_DELAY;
    };
    if !auto || !widget.is_visible().unwrap_or(false) {
        return IDLE_DELAY;
    }
    match sample(&widget) {
        Ok(Some(avg)) => {
            let n = EMITTED.fetch_add(1, Ordering::Relaxed);
            // first value, then about once a minute, so the log shows the sampler is alive
            if n.is_multiple_of(20) {
                log::info!("{EVENT_BG} {avg:.3} (sample {})", n + 1);
            } else {
                log::debug!("{EVENT_BG} {avg:.3}");
            }
            let _ = app.emit_to(WIDGET, EVENT_BG, avg);
        }
        Ok(None) => {}
        Err(e) => {
            // Per-tick errors are swallowed; only the first one is logged.
            if !FAILED_ONCE.swap(true, Ordering::Relaxed) {
                log::warn!("theme sampler failed (further failures not logged): {e}");
            }
        }
    }
    ACTIVE_DELAY
}

/// Start the sampler on its own thread for the life of the app.
pub fn start_sampler(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("theme-sampler".into())
        .spawn(move || loop {
            let delay = tick(&app);
            std::thread::sleep(delay);
        });
    if let Err(e) = spawned {
        log::warn!("could not start the theme sampler: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_skips_the_widget() {
        // 4x2 area: left half white, right half black; the black half is the "widget".
        let area = Rect {
            x0: 10,
            y0: 20,
            x1: 14,
            y1: 22,
        };
        let skip = Rect {
            x0: 12,
            y0: 20,
            x1: 14,
            y1: 22,
        };
        let mut buf = Vec::new();
        for _row in 0..2 {
            for col in 0..4 {
                let v = if col < 2 { 255 } else { 0 };
                buf.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let all = average_luminance(
            &buf,
            area,
            Rect {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
            1,
        )
        .unwrap();
        assert!((all - 0.5).abs() < 1e-9);
        let around = average_luminance(&buf, area, skip, 1).unwrap();
        assert!((around - 1.0).abs() < 1e-9);
        assert_eq!(average_luminance(&buf, area, area, 1), None);
    }

    #[test]
    fn capture_area_is_clamped_to_the_monitor() {
        let display = Rect {
            x0: 0,
            y0: 0,
            x1: 1920,
            y1: 1080,
        };
        let outer = Rect {
            x0: -48,
            y0: 1000,
            x1: 238,
            y1: 1200,
        }
        .clamp_to(display);
        assert_eq!(
            outer,
            Rect {
                x0: 0,
                y0: 1000,
                x1: 238,
                y1: 1080
            }
        );
    }
}
