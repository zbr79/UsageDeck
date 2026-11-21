//! Pure-Rust native Win32 Settings window for UsageDeck (zero WebView2 / zero Tauri).
//!
//! Provides:
//! - Provider enable/disable toggles (Claude, Cursor, Codex, Grok Bot, OpenCode)
//! - General options ("Start with Windows" + "Hide in fullscreen apps")
//! - OpenCode API key management backed by `Store` + Windows Credential Manager
//!   (list with key hints, Add/Replace with `oc_sk_`/`sk-` validation, Copy Key, Remove)
//! - Version readout, "Check for updates", "Open Data Folder", and "Open Log Folder"
//! - Open-state tracking (`is_settings_open`) so the auto-updater defers restarts while open.

use crate::autostart;
use crate::clipboard;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use usagedeck_core::store::{
    key_hint, Account, CredentialVault, Settings, Store, OPENCODE_KEY_PREFIXES,
};
use usagedeck_core::updater::release::{is_newer, parse_release, Version};
use usagedeck_core::{default_data_dir, default_log_dir, Core};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetMonitorInfoW, GetSysColorBrush, MonitorFromWindow,
    SetBkMode, CLEARTYPE_QUALITY, COLOR_BTNFACE, DEFAULT_CHARSET, HDC, HFONT, HGDIOBJ, LOGFONTW,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowTextLengthW,
    GetWindowTextW, LoadCursorW, LoadIconW, PostMessageW, RegisterClassExW, SendMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, HMENU, IDC_ARROW, IDI_APPLICATION, SW_RESTORE,
    SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_KEYDOWN, WM_NCDESTROY,
    WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_DLGMODALFRAME, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

// Standard Win32 child control style/message constants (frozen Win32 ABI).
const BS_PUSHBUTTON: u32 = 0x0000_0000;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BS_GROUPBOX: u32 = 0x0000_0007;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_UNCHECKED: usize = 0;
const BST_CHECKED: usize = 1;
const BN_CLICKED: u16 = 0;

const ES_PASSWORD: u32 = 0x0020;
const ES_AUTOHSCROLL: u32 = 0x0080;

const LBS_NOTIFY: u32 = 0x0001;
const LBS_NOINTEGRALHEIGHT: u32 = 0x0100;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_SETCURSEL: u32 = 0x0186;
const LB_GETCURSEL: u32 = 0x0188;
const LBN_SELCHANGE: u16 = 1;

const SS_LEFT: u32 = 0x0000_0000;
const SS_ENDELLIPSIS: u32 = 0x0000_4000;

const WM_SETFONT: u32 = 0x0030;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_APP_UPDATE_STATUS: u32 = WM_APP + 20;

// Child control IDs.
const ID_CHK_CLAUDE: u16 = 2001;
const ID_CHK_CURSOR: u16 = 2002;
const ID_CHK_CODEX: u16 = 2003;
const ID_CHK_GROK: u16 = 2004;
const ID_CHK_OPENCODE: u16 = 2005;
const ID_CHK_START_LOGIN: u16 = 2010;
const ID_CHK_HIDE_FS: u16 = 2011;

const ID_LIST_ACCOUNTS: u16 = 2020;
const ID_BTN_COPY_KEY: u16 = 2021;
const ID_BTN_REMOVE_ACCOUNT: u16 = 2022;
const ID_EDIT_EMAIL: u16 = 2023;
const ID_EDIT_KEY: u16 = 2024;
const ID_BTN_ADD_ACCOUNT: u16 = 2025;
const ID_LBL_ACCOUNT_STATUS: u16 = 2026;

const ID_LBL_UPDATE_STATUS: u16 = 2030;
const ID_BTN_CHECK_UPDATES: u16 = 2031;
const ID_BTN_OPEN_DATA: u16 = 2032;
const ID_BTN_OPEN_LOGS: u16 = 2033;
const ID_BTN_CLOSE: u16 = 2040;

const MAX_EMAIL_LENGTH: usize = 320;
const MAX_KEY_LENGTH: usize = 512;
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/zbr79/UsageDeck/releases/latest";
const USER_AGENT: &str = "UsageDeck-updater";

const CLIENT_W_DIP: f32 = 460.0;
const CLIENT_H_DIP: f32 = 570.0;

static CLASS_REGISTERED: Once = Once::new();
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static PENDING_UPDATE_TEXT: Mutex<Option<(String, bool)>> = Mutex::new(None);

struct Controls {
    chk_claude: HWND,
    chk_cursor: HWND,
    chk_codex: HWND,
    chk_grok: HWND,
    chk_opencode: HWND,
    chk_start_login: HWND,
    chk_hide_fs: HWND,
    list_accounts: HWND,
    btn_copy_key: HWND,
    btn_remove_account: HWND,
    edit_email: HWND,
    edit_key: HWND,
    lbl_account_status: HWND,
    lbl_update_status: HWND,
    btn_check_updates: HWND,
}

struct SettingsState {
    controls: Controls,
    fonts: Vec<HFONT>,
    store: Option<Store>,
    rt: Option<tokio::runtime::Runtime>,
    accounts: Vec<Account>,
    on_changed: Arc<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Arc<dyn Fn(HWND) + Send + Sync>>,
}

impl Drop for SettingsState {
    fn drop(&mut self) {
        // SAFETY: destroys GDI fonts created for this dialog window.
        unsafe {
            for font in &self.fonts {
                if !font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
            }
        }
    }
}

thread_local! {
    static SETTINGS_STATE: RefCell<Option<SettingsState>> = const { RefCell::new(None) };
}

/// Returns `true` while the UsageDeck Settings window is open.
///
/// The background auto-updater checks this to defer silent restarts while the
/// user is editing preferences or API keys.
pub fn is_settings_open() -> bool {
    SETTINGS_OPEN.load(Ordering::Acquire)
}

/// Validate an OpenCode API key against the `oc_sk_` / `sk-` prefix rules from `usagedeck-core`.
pub fn is_valid_opencode_key(key: &str) -> bool {
    OPENCODE_KEY_PREFIXES
        .iter()
        .any(|p| key.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
        && key.encode_utf16().count() <= MAX_KEY_LENGTH
        && !key.chars().any(char::is_whitespace)
}

/// Update the version/update status label on an open Settings window from any thread.
pub fn set_update_status_text(text: &str) {
    post_update_status(text.to_string(), true);
}

fn post_update_status(text: String, enable_button: bool) {
    if let Ok(mut slot) = PENDING_UPDATE_TEXT.lock() {
        *slot = Some((text, enable_button));
    }
    let raw = SETTINGS_HWND.load(Ordering::Acquire);
    if raw != 0 {
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is thread-safe for posting custom `WM_APP` notifications.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE_STATUS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Open the native Win32 UsageDeck Settings window (or bring the existing one to the front).
///
/// `on_changed` is invoked whenever provider toggles, behavior settings, or OpenCode
/// accounts are saved. When `on_check_updates` is `None`, the dialog performs a built-in
/// GitHub latest-release check in a background thread.
pub fn open_settings_dialog_with_updater(
    owner: HWND,
    on_changed: Box<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Box<dyn Fn(HWND) + Send + Sync>>,
) {
    // Single-instance behavior: bring the open Settings window to front.
    let existing = SETTINGS_HWND.load(Ordering::Acquire);
    if existing != 0 {
        let hwnd = HWND(existing as *mut std::ffi::c_void);
        // SAFETY: restores and focuses our own live Settings window.
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }

    let on_changed: Arc<dyn Fn() + Send + Sync> = Arc::from(on_changed);
    let on_check_updates: Option<Arc<dyn Fn(HWND) + Send + Sync>> = on_check_updates.map(Arc::from);

    // SAFETY: Win32 class registration and native control creation on the UI thread.
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            log::warn!("settings_dialog: GetModuleHandleW failed");
            return;
        };
        let class_name = w!("UsageDeckSettingsWindow");
        CLASS_REGISTERED.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(settings_wnd_proc),
                hInstance: hinstance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        });

        let dpi = if !owner.is_invalid() {
            let d = GetDpiForWindow(owner);
            if d == 0 {
                96
            } else {
                d
            }
        } else {
            96
        };
        let scale = dpi as f32 / 96.0;
        let px = |dip: f32| -> i32 { (dip * scale).round() as i32 };

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let ex_style = WS_EX_DLGMODALFRAME;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: px(CLIENT_W_DIP),
            bottom: px(CLIENT_H_DIP),
        };
        let _ = AdjustWindowRectEx(&mut rect, style, false, ex_style);
        let win_w = rect.right - rect.left;
        let win_h = rect.bottom - rect.top;

        // Center on the monitor hosting `owner` (or primary monitor).
        let mon = MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST);
        let mut mon_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let (x, y) = if GetMonitorInfoW(mon, &mut mon_info).as_bool() {
            let wa = mon_info.rcWork;
            (
                wa.left + ((wa.right - wa.left - win_w) / 2).max(0),
                wa.top + ((wa.bottom - wa.top - win_h) / 2).max(0),
            )
        } else {
            (120, 120)
        };

        let Ok(hwnd) = CreateWindowExW(
            ex_style,
            class_name,
            w!("UsageDeck Settings"),
            style,
            x,
            y,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            log::warn!("settings_dialog: CreateWindowExW failed");
            return;
        };

        let normal_font = make_ui_font(-px(13.5), 400);
        let bold_font = make_ui_font(-px(13.5), 600);
        let small_font = make_ui_font(-px(12.0), 400);

        let create_ctrl = |ex: WINDOW_EX_STYLE,
                           cls: PCWSTR,
                           text: &str,
                           st: WINDOW_STYLE,
                           cx: f32,
                           cy: f32,
                           cw: f32,
                           ch: f32,
                           id: u16,
                           font: HFONT|
         -> HWND {
            let wide = wide(text);
            let hmenu = if id == 0 {
                None
            } else {
                Some(HMENU(usize::from(id) as *mut std::ffi::c_void))
            };
            let ctrl = CreateWindowExW(
                ex,
                cls,
                PCWSTR(wide.as_ptr()),
                st,
                px(cx),
                px(cy),
                px(cw),
                px(ch),
                Some(hwnd),
                hmenu,
                Some(hinstance.into()),
                None,
            )
            .unwrap_or_default();
            if !ctrl.is_invalid() && !font.is_invalid() {
                SendMessageW(
                    ctrl,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            ctrl
        };

        let no_ex = WINDOW_EX_STYLE(0);
        let group_st = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_GROUPBOX);
        let chk_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX);
        let btn_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON);
        let def_btn_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON);
        let lbl_st = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT | SS_ENDELLIPSIS);

        // ---- Section 1: Providers ----
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Providers",
            group_st,
            16.0,
            12.0,
            428.0,
            82.0,
            0,
            bold_font,
        );
        let chk_claude = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Claude",
            chk_st,
            28.0,
            36.0,
            120.0,
            22.0,
            ID_CHK_CLAUDE,
            normal_font,
        );
        let chk_cursor = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Cursor",
            chk_st,
            160.0,
            36.0,
            120.0,
            22.0,
            ID_CHK_CURSOR,
            normal_font,
        );
        let chk_codex = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Codex",
            chk_st,
            292.0,
            36.0,
            120.0,
            22.0,
            ID_CHK_CODEX,
            normal_font,
        );
        let chk_grok = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Grok Bot",
            chk_st,
            28.0,
            64.0,
            120.0,
            22.0,
            ID_CHK_GROK,
            normal_font,
        );
        let chk_opencode = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "OpenCode",
            chk_st,
            160.0,
            64.0,
            120.0,
            22.0,
            ID_CHK_OPENCODE,
            normal_font,
        );

        // ---- Section 2: General ----
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "General",
            group_st,
            16.0,
            104.0,
            428.0,
            76.0,
            0,
            bold_font,
        );
        let chk_start_login = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Start with Windows",
            chk_st,
            28.0,
            126.0,
            380.0,
            22.0,
            ID_CHK_START_LOGIN,
            normal_font,
        );
        let chk_hide_fs = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Hide in fullscreen apps",
            chk_st,
            28.0,
            152.0,
            380.0,
            22.0,
            ID_CHK_HIDE_FS,
            normal_font,
        );

        // ---- Section 3: OpenCode API Keys ----
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "OpenCode API Keys",
            group_st,
            16.0,
            190.0,
            428.0,
            242.0,
            0,
            bold_font,
        );
        let list_accounts = create_ctrl(
            WS_EX_CLIENTEDGE,
            w!("LISTBOX"),
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | WINDOW_STYLE(LBS_NOTIFY | LBS_NOINTEGRALHEIGHT),
            28.0,
            214.0,
            404.0,
            84.0,
            ID_LIST_ACCOUNTS,
            normal_font,
        );
        let btn_copy_key = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Copy Key",
            btn_st,
            28.0,
            304.0,
            96.0,
            26.0,
            ID_BTN_COPY_KEY,
            normal_font,
        );
        let btn_remove_account = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Remove Account",
            btn_st,
            132.0,
            304.0,
            124.0,
            26.0,
            ID_BTN_REMOVE_ACCOUNT,
            normal_font,
        );

        let _ = create_ctrl(
            no_ex,
            w!("STATIC"),
            "Email:",
            lbl_st,
            28.0,
            343.0,
            52.0,
            20.0,
            0,
            normal_font,
        );
        let edit_email = create_ctrl(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL),
            84.0,
            340.0,
            348.0,
            24.0,
            ID_EDIT_EMAIL,
            normal_font,
        );
        let _ = create_ctrl(
            no_ex,
            w!("STATIC"),
            "API Key:",
            lbl_st,
            28.0,
            373.0,
            52.0,
            20.0,
            0,
            normal_font,
        );
        let edit_key = create_ctrl(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL | ES_PASSWORD),
            84.0,
            370.0,
            246.0,
            24.0,
            ID_EDIT_KEY,
            normal_font,
        );
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Add / Save",
            def_btn_st,
            338.0,
            369.0,
            94.0,
            26.0,
            ID_BTN_ADD_ACCOUNT,
            normal_font,
        );
        let lbl_account_status = create_ctrl(
            no_ex,
            w!("STATIC"),
            "Keys are stored in Windows Credential Manager (oc_sk_… or sk-…).",
            lbl_st,
            28.0,
            404.0,
            404.0,
            20.0,
            ID_LBL_ACCOUNT_STATUS,
            small_font,
        );

        // ---- Section 4: About & Maintenance ----
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "About & Diagnostics",
            group_st,
            16.0,
            442.0,
            428.0,
            82.0,
            0,
            bold_font,
        );
        let version_label = format!("UsageDeck v{}", env!("CARGO_PKG_VERSION"));
        let lbl_update_status = create_ctrl(
            no_ex,
            w!("STATIC"),
            &version_label,
            lbl_st,
            28.0,
            464.0,
            404.0,
            20.0,
            ID_LBL_UPDATE_STATUS,
            normal_font,
        );
        let btn_check_updates = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Check for updates",
            btn_st,
            28.0,
            488.0,
            134.0,
            26.0,
            ID_BTN_CHECK_UPDATES,
            normal_font,
        );
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Open Data Folder",
            btn_st,
            170.0,
            488.0,
            128.0,
            26.0,
            ID_BTN_OPEN_DATA,
            normal_font,
        );
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Open Log Folder",
            btn_st,
            306.0,
            488.0,
            126.0,
            26.0,
            ID_BTN_OPEN_LOGS,
            normal_font,
        );

        // ---- Bottom Close button ----
        let _ = create_ctrl(
            no_ex,
            w!("BUTTON"),
            "Close",
            btn_st,
            356.0,
            534.0,
            88.0,
            26.0,
            ID_BTN_CLOSE,
            normal_font,
        );

        let store = default_data_dir().map(|dir| Store::new(dir, Box::new(CredentialVault)));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok();

        let mut state = SettingsState {
            controls: Controls {
                chk_claude,
                chk_cursor,
                chk_codex,
                chk_grok,
                chk_opencode,
                chk_start_login,
                chk_hide_fs,
                list_accounts,
                btn_copy_key,
                btn_remove_account,
                edit_email,
                edit_key,
                lbl_account_status,
                lbl_update_status,
                btn_check_updates,
            },
            fonts: vec![normal_font, bold_font, small_font],
            store,
            rt,
            accounts: Vec::new(),
            on_changed,
            on_check_updates,
        };

        load_settings_into_controls(&state);
        reload_accounts(&mut state, None);

        SETTINGS_OPEN.store(true, Ordering::Release);
        SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Release);
        SETTINGS_STATE.with(|cell| {
            *cell.borrow_mut() = Some(state);
        });

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn make_ui_font(height: i32, weight: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: height,
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (dst, src) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
        *dst = src;
    }
    CreateFontIndirectW(&lf)
}

fn set_control_text(hwnd: HWND, text: &str) {
    let w = wide(text);
    // SAFETY: `w` is NUL-terminated and outlives the `SetWindowTextW` call.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

fn get_control_text(hwnd: HWND) -> String {
    // SAFETY: reads UTF-16 text from our own Edit control into a correctly sized buffer.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buf);
        if copied <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..copied as usize])
    }
}

fn set_checked(hwnd: HWND, checked: bool) {
    let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
    // SAFETY: standard checkbox `BM_SETCHECK` on our own control.
    unsafe {
        SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(state)), None);
    }
}

fn is_checked(hwnd: HWND) -> bool {
    // SAFETY: standard checkbox `BM_GETCHECK` on our own control.
    unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None).0 as usize == BST_CHECKED }
}

fn load_settings_into_controls(state: &SettingsState) {
    let settings = state
        .store
        .as_ref()
        .map(Store::get_settings)
        .unwrap_or_default();
    set_checked(state.controls.chk_claude, settings.claude_enabled);
    set_checked(state.controls.chk_cursor, settings.cursor_enabled);
    set_checked(state.controls.chk_codex, settings.codex_enabled);
    set_checked(state.controls.chk_grok, settings.grok_enabled);
    set_checked(state.controls.chk_opencode, settings.opencode_enabled);
    set_checked(state.controls.chk_start_login, settings.start_at_login);
    set_checked(state.controls.chk_hide_fs, settings.hide_in_fullscreen);
}

fn reload_accounts(state: &mut SettingsState, select_email: Option<&str>) {
    state.accounts = match (&state.store, &state.rt) {
        (Some(store), Some(rt)) => rt.block_on(store.get_accounts()),
        _ => Vec::new(),
    };

    // SAFETY: updates our own ListBox control items.
    unsafe {
        SendMessageW(state.controls.list_accounts, LB_RESETCONTENT, None, None);
        let mut selected_idx: Option<usize> = None;
        for (idx, acc) in state.accounts.iter().enumerate() {
            let hint = if acc.key.is_empty() {
                "missing key".to_string()
            } else {
                key_hint(&acc.key)
            };
            let row = wide(&format!("{}   ({hint})", acc.email));
            SendMessageW(
                state.controls.list_accounts,
                LB_ADDSTRING,
                None,
                Some(LPARAM(row.as_ptr() as isize)),
            );
            if let Some(target) = select_email {
                if acc.email.eq_ignore_ascii_case(target) {
                    selected_idx = Some(idx);
                }
            }
        }
        if let Some(idx) = selected_idx {
            SendMessageW(
                state.controls.list_accounts,
                LB_SETCURSEL,
                Some(WPARAM(idx)),
                None,
            );
        }
    }
    sync_account_buttons(state);
}

fn selected_account_index(state: &SettingsState) -> Option<usize> {
    // SAFETY: queries current selection index from our own ListBox.
    let sel = unsafe { SendMessageW(state.controls.list_accounts, LB_GETCURSEL, None, None).0 };
    if sel < 0 {
        None
    } else {
        let idx = sel as usize;
        (idx < state.accounts.len()).then_some(idx)
    }
}

fn sync_account_buttons(state: &SettingsState) {
    let has_sel = selected_account_index(state).is_some();
    // SAFETY: enables/disables our own child buttons.
    unsafe {
        let _ = EnableWindow(state.controls.btn_copy_key, has_sel);
        let _ = EnableWindow(state.controls.btn_remove_account, has_sel);
    }
}

fn save_settings_change(
    state: &SettingsState,
    change: impl FnOnce(Settings) -> Settings,
) -> Result<Settings, String> {
    let (Some(store), Some(rt)) = (&state.store, &state.rt) else {
        return Err("Data folder unavailable (APPDATA is not set).".to_string());
    };
    rt.block_on(store.update_settings(change))
        .map_err(|e| format!("Failed to save settings: {e}"))
}

fn open_folder(dir: Option<PathBuf>) {
    let Some(dir) = dir else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
}

fn run_builtin_update_check() {
    let _ = std::thread::Builder::new()
        .name("usagedeck-update-check".into())
        .spawn(|| {
            let current_raw = env!("CARGO_PKG_VERSION");
            let current_ver: Option<Version> = current_raw
                .split('-')
                .next()
                .and_then(|base| base.parse().ok());

            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    post_update_status(format!("Update check failed: {e}"), true);
                    return;
                }
            };

            let data_dir = default_data_dir().unwrap_or_else(std::env::temp_dir);
            let core = Core::new(Store::new(data_dir, Box::new(CredentialVault)));

            let outcome = rt.block_on(async {
                let resp = core
                    .http
                    .get(LATEST_RELEASE_URL)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .map_err(|e| format!("network error: {e}"))?;
                let resp = resp
                    .error_for_status()
                    .map_err(|e| format!("GitHub error: {e}"))?;
                let body = resp.text().await.map_err(|e| format!("read error: {e}"))?;
                parse_release(&body).map_err(|e| e.to_string())
            });

            match outcome {
                Ok(release) => {
                    if let Some(cur) = current_ver {
                        if is_newer(cur, release.version) {
                            post_update_status(
                                format!(
                                    "Update v{} available (current v{current_raw})",
                                    release.version
                                ),
                                true,
                            );
                            return;
                        }
                    }
                    post_update_status(
                        format!(
                            "You're up to date — v{current_raw} (latest v{})",
                            release.version
                        ),
                        true,
                    );
                }
                Err(msg) => {
                    post_update_status(format!("Couldn't check for updates ({msg})"), true);
                }
            }
        });
}

fn handle_command(hwnd: HWND, id: u16, notify: u16) {
    if id == ID_LIST_ACCOUNTS && notify == LBN_SELCHANGE {
        SETTINGS_STATE.with(|cell| {
            if let Some(state) = cell.borrow().as_ref() {
                sync_account_buttons(state);
            }
        });
        return;
    }

    if notify != BN_CLICKED {
        return;
    }

    // Collect any callback or close request outside the RefCell borrow to avoid re-entrancy.
    let mut notify_changed: Option<Arc<dyn Fn() + Send + Sync>> = None;
    let mut custom_update_cb: Option<Arc<dyn Fn(HWND) + Send + Sync>> = None;
    let mut run_builtin_update = false;
    let mut should_close = false;

    SETTINGS_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return;
        };

        match id {
            ID_CHK_CLAUDE | ID_CHK_CURSOR | ID_CHK_CODEX | ID_CHK_GROK | ID_CHK_OPENCODE
            | ID_CHK_START_LOGIN | ID_CHK_HIDE_FS => {
                let claude = is_checked(state.controls.chk_claude);
                let cursor = is_checked(state.controls.chk_cursor);
                let codex = is_checked(state.controls.chk_codex);
                let grok = is_checked(state.controls.chk_grok);
                let opencode = is_checked(state.controls.chk_opencode);
                let start_login = is_checked(state.controls.chk_start_login);
                let hide_fs = is_checked(state.controls.chk_hide_fs);

                match save_settings_change(state, |mut s| {
                    s.claude_enabled = claude;
                    s.cursor_enabled = cursor;
                    s.codex_enabled = codex;
                    s.grok_enabled = grok;
                    s.opencode_enabled = opencode;
                    s.start_at_login = start_login;
                    s.hide_in_fullscreen = hide_fs;
                    s
                }) {
                    Ok(_) => {
                        if id == ID_CHK_START_LOGIN {
                            autostart::set_start_at_login(start_login);
                        }
                        notify_changed = Some(state.on_changed.clone());
                    }
                    Err(e) => {
                        log::warn!("{e}");
                        set_control_text(state.controls.lbl_account_status, &e);
                    }
                }
            }
            ID_BTN_ADD_ACCOUNT => {
                let email = get_control_text(state.controls.edit_email)
                    .trim()
                    .to_string();
                let key = get_control_text(state.controls.edit_key).trim().to_string();

                if email.is_empty() || email.encode_utf16().count() > MAX_EMAIL_LENGTH {
                    set_control_text(
                        state.controls.lbl_account_status,
                        "Enter an email or label (up to 320 characters).",
                    );
                    // SAFETY: focus email input on validation failure.
                    unsafe {
                        let _ = SetFocus(Some(state.controls.edit_email));
                    }
                    return;
                }
                if !is_valid_opencode_key(&key) {
                    set_control_text(
                        state.controls.lbl_account_status,
                        "API key must start with oc_sk_ or sk- with no spaces.",
                    );
                    // SAFETY: focus key input on validation failure.
                    unsafe {
                        let _ = SetFocus(Some(state.controls.edit_key));
                    }
                    return;
                }

                let save_res = match (&state.store, &state.rt) {
                    (Some(store), Some(rt)) => {
                        let email_c = email.clone();
                        let key_c = key.clone();
                        rt.block_on(store.update_accounts(|mut accounts| {
                            let lower = email_c.to_lowercase();
                            match accounts
                                .iter_mut()
                                .find(|a| a.email.to_lowercase() == lower)
                            {
                                Some(existing) => existing.key = key_c,
                                None => accounts.push(Account {
                                    email: email_c,
                                    key: key_c,
                                }),
                            }
                            accounts
                        }))
                        .map_err(|e| format!("Could not save OpenCode account: {e}"))
                    }
                    _ => Err("Data folder unavailable (APPDATA is not set).".to_string()),
                };

                match save_res {
                    Ok(_) => {
                        set_control_text(state.controls.edit_email, "");
                        set_control_text(state.controls.edit_key, "");
                        reload_accounts(state, Some(&email));
                        set_control_text(
                            state.controls.lbl_account_status,
                            &format!("Saved OpenCode key for {email}."),
                        );
                        notify_changed = Some(state.on_changed.clone());
                    }
                    Err(e) => {
                        log::warn!("{e}");
                        set_control_text(state.controls.lbl_account_status, &e);
                    }
                }
            }
            ID_BTN_COPY_KEY => {
                let Some(idx) = selected_account_index(state) else {
                    return;
                };
                let acc = &state.accounts[idx];
                if acc.key.is_empty() {
                    set_control_text(
                        state.controls.lbl_account_status,
                        &format!("No key stored in Credential Manager for {}.", acc.email),
                    );
                    return;
                }
                match clipboard::copy_text(&acc.key) {
                    Ok(()) => {
                        set_control_text(
                            state.controls.lbl_account_status,
                            &format!("Copied API key for {}.", acc.email),
                        );
                    }
                    Err(e) => {
                        log::warn!("copy_text failed: {e}");
                        set_control_text(
                            state.controls.lbl_account_status,
                            &format!("Clipboard error: {e}"),
                        );
                    }
                }
            }
            ID_BTN_REMOVE_ACCOUNT => {
                let Some(idx) = selected_account_index(state) else {
                    return;
                };
                let email = state.accounts[idx].email.clone();
                let lower = email.to_lowercase();

                let remove_res = match (&state.store, &state.rt) {
                    (Some(store), Some(rt)) => {
                        let first = rt.block_on(store.update_accounts(|accounts| {
                            accounts
                                .into_iter()
                                .filter(|a| a.email.to_lowercase() != lower)
                                .collect()
                        }));
                        match first {
                            Ok(_) => rt
                                .block_on(store.forget_account(&email))
                                .map(|_| ())
                                .map_err(|e| format!("Failed to clean account settings: {e}")),
                            Err(e) => Err(format!("Failed to remove account: {e}")),
                        }
                    }
                    _ => Err("Data folder unavailable (APPDATA is not set).".to_string()),
                };

                match remove_res {
                    Ok(()) => {
                        reload_accounts(state, None);
                        set_control_text(
                            state.controls.lbl_account_status,
                            &format!("Removed {email}."),
                        );
                        notify_changed = Some(state.on_changed.clone());
                    }
                    Err(e) => {
                        log::warn!("{e}");
                        set_control_text(state.controls.lbl_account_status, &e);
                    }
                }
            }
            ID_BTN_CHECK_UPDATES => {
                if let Some(ref cb) = state.on_check_updates {
                    set_control_text(state.controls.lbl_update_status, "Checking for updates...");
                    custom_update_cb = Some(cb.clone());
                } else {
                    set_control_text(state.controls.lbl_update_status, "Checking for updates...");
                    // SAFETY: disable button while background check runs.
                    unsafe {
                        let _ = EnableWindow(state.controls.btn_check_updates, false);
                    }
                    run_builtin_update = true;
                }
            }
            ID_BTN_OPEN_DATA => open_folder(default_data_dir()),
            ID_BTN_OPEN_LOGS => open_folder(default_log_dir()),
            ID_BTN_CLOSE => should_close = true,
            _ => {}
        }
    });

    if let Some(cb) = notify_changed {
        cb();
    }
    if let Some(cb) = custom_update_cb {
        cb(hwnd);
    }
    if run_builtin_update {
        run_builtin_update_check();
    }
    if should_close {
        // SAFETY: closes our own Settings window cleanly without quitting the app.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as u16;
            let notify = ((wparam.0 >> 16) & 0xffff) as u16;
            handle_command(hwnd, id, notify);
            LRESULT(0)
        }
        WM_APP_UPDATE_STATUS => {
            let pending = PENDING_UPDATE_TEXT
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some((text, enable_btn)) = pending {
                SETTINGS_STATE.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        set_control_text(state.controls.lbl_update_status, &text);
                        let _ = EnableWindow(state.controls.btn_check_updates, enable_btn);
                    }
                });
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut std::ffi::c_void);
            SetBkMode(hdc, TRANSPARENT);
            LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
        }
        WM_KEYDOWN => {
            if wparam.0 == usize::from(VK_ESCAPE.0) {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SETTINGS_OPEN.store(false, Ordering::Release);
            SETTINGS_HWND.store(0, Ordering::Release);
            SETTINGS_STATE.with(|cell| {
                let _ = cell.borrow_mut().take();
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
