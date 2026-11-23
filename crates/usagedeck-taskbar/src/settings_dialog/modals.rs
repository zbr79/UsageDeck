//! Clean native Win32 modal sub-dialogs for text input and confirmation.
//!
//! Using native Win32 `EDIT` controls inside centered, dark/light-themed modal
//! popup windows preserves full Windows IME composition, Ctrl+V paste, password
//! masking, and keyboard navigation (`Tab`, `Enter`, `Esc`).
//!
//! Every public function here MUST be called strictly outside any `SETTINGS_STATE`
//! `RefCell` borrow because each modal runs its own nested Win32 message loop.

use crate::settings_dialog::is_valid_opencode_key;
use crate::settings_dialog::theme::apply_titlebar_theme;
use std::cell::RefCell;
use std::sync::Once;
use usagedeck_core::store::{normalize_ymd_date, MAX_NICKNAME_LENGTH};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, FillRect, InvalidateRect, SetBkColor,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET, HBRUSH, HDC, HFONT, HGDIOBJ,
    LOGFONTW, OPAQUE, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    IsDialogMessageW, LoadCursorW, MessageBoxW, RegisterClassExW, SendMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage, HMENU, IDC_ARROW, IDYES,
    MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO, MSG, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_ERASEBKGND, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const BS_PUSHBUTTON: u32 = 0x0000_0000;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BM_GETCHECK: u32 = 0x00F0;
const BST_CHECKED: usize = 1;
const BN_CLICKED: u16 = 0;

const ES_PASSWORD: u32 = 0x0020;
const ES_AUTOHSCROLL: u32 = 0x0080;
const EM_SETPASSWORDCHAR: u32 = 0x00CC;
const EM_SETSEL: u32 = 0x00B1;

const SS_LEFT: u32 = 0x0000_0000;
const SS_ENDELLIPSIS: u32 = 0x0000_4000;

const WM_SETFONT: u32 = 0x0030;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLORSTATIC: u32 = 0x0138;

const ID_OK: u16 = 1;
const ID_CANCEL: u16 = 2;
const ID_INPUT_1: u16 = 101;
const ID_INPUT_2: u16 = 102;
const ID_CHK_SHOW_KEY: u16 = 103;
const ID_LBL_ERROR: u16 = 104;

const MAX_EMAIL_LENGTH: usize = 320;

static MODAL_CLASS_REG: Once = Once::new();

#[derive(Clone, Copy)]
enum ModalKind {
    Rename,
    AddOpenCode,
    ReplaceOpenCode,
    HourlyTarget,
    SubscriptionEnd,
}

#[derive(Clone, Debug)]
enum ModalOutcome {
    Rename(String),
    OpenCode(String, String),
    HourlyTarget(Option<u32>),
    SubscriptionEnd(Option<String>),
}

struct ModalState {
    kind: ModalKind,
    locked_email: Option<String>,
    is_dark: bool,
    bg_color: COLORREF,
    input_bg: COLORREF,
    fg_color: COLORREF,
    err_color: COLORREF,
    bg_brush: HBRUSH,
    input_brush: HBRUSH,
    fonts: Vec<HFONT>,
    edit1: HWND,
    edit2: HWND,
    chk_show: HWND,
    lbl_error: HWND,
    done: bool,
    outcome: Option<ModalOutcome>,
}

impl Drop for ModalState {
    fn drop(&mut self) {
        // SAFETY: deletes GDI brushes and fonts owned exclusively by this modal dialog.
        unsafe {
            if !self.bg_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
            }
            if !self.input_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.input_brush.0));
            }
            for f in &self.fonts {
                if !f.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(f.0));
                }
            }
        }
    }
}

thread_local! {
    static ACTIVE_MODAL: RefCell<Option<ModalState>> = const { RefCell::new(None) };
}

struct OwnerDisableGuard(HWND);

impl OwnerDisableGuard {
    fn new(owner: HWND) -> Self {
        if !owner.is_invalid() {
            // SAFETY: disables owner window for the duration of the modal loop.
            unsafe {
                let _ = EnableWindow(owner, false);
            }
        }
        Self(owner)
    }
}

impl Drop for OwnerDisableGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: re-enables and restores focus to owner window when modal closes.
            unsafe {
                let _ = EnableWindow(self.0, true);
                let _ = SetForegroundWindow(self.0);
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn colorref(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

unsafe fn make_font(height_px: i32, weight: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: height_px,
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

fn get_edit_text(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }
    // SAFETY: queries UTF-16 text from our own child Edit control.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn set_ctrl_text(hwnd: HWND, text: &str) {
    if hwnd.is_invalid() {
        return;
    }
    let w = wide(text);
    // SAFETY: `w` is NUL-terminated and outlives `SetWindowTextW`.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

/// Confirm removing an account (`No` is the default button).
pub fn confirm_remove_account(
    owner: HWND,
    provider_name: &str,
    display_name: &str,
    is_opencode: bool,
) -> bool {
    let body = if is_opencode {
        format!(
            "Remove OpenCode account \"{display_name}\"?\n\nIts API key will be deleted from Windows Credential Manager."
        )
    } else {
        format!(
            "Remove {provider_name} account \"{display_name}\"?\n\nIt will be removed from UsageDeck's remembered accounts and cached usage. If you are still signed in to {provider_name}, it may reappear on the next refresh."
        )
    };
    let msg = wide(&body);
    let title = wide("Remove account");
    // SAFETY: NUL-terminated UTF-16 strings; runs modal confirmation outside any RefCell borrow.
    unsafe {
        MessageBoxW(
            Some(owner),
            PCWSTR(msg.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) == IDYES
    }
}

/// Prompt the user to rename an account (empty clears the nickname).
pub fn prompt_rename(
    owner: HWND,
    is_dark: bool,
    account_label: &str,
    current_nickname: &str,
) -> Option<String> {
    let desc = format!("Nickname for {account_label} (leave blank to reset):");
    match run_modal(
        owner,
        is_dark,
        ModalKind::Rename,
        "Rename account",
        &desc,
        current_nickname,
        None,
        420.0,
        166.0,
    ) {
        Some(ModalOutcome::Rename(v)) => Some(v),
        _ => None,
    }
}

/// Prompt the user to record or clear a subscription end date (`YYYY-MM-DD`, blank clears).
pub fn prompt_subscription_end(
    owner: HWND,
    is_dark: bool,
    account_label: &str,
    current_date: Option<&str>,
) -> Option<Option<String>> {
    let desc =
        format!("Subscription end date for {account_label} (YYYY-MM-DD, leave blank to clear):");
    match run_modal(
        owner,
        is_dark,
        ModalKind::SubscriptionEnd,
        "Subscription end date",
        &desc,
        current_date.unwrap_or(""),
        None,
        440.0,
        172.0,
    ) {
        Some(ModalOutcome::SubscriptionEnd(v)) => Some(v),
        _ => None,
    }
}

/// Prompt to add a new OpenCode account or replace an existing account's API key.
pub fn prompt_opencode_key(
    owner: HWND,
    is_dark: bool,
    existing_email: Option<&str>,
) -> Option<(String, String)> {
    let (kind, title, desc, h) = match existing_email {
        Some(email) => (
            ModalKind::ReplaceOpenCode,
            "Replace API key",
            format!("New OpenCode API key for {email} (oc_sk_… or sk-…):"),
            204.0,
        ),
        None => (
            ModalKind::AddOpenCode,
            "Add OpenCode account",
            "Enter an email/label and your OpenCode Go API key (oc_sk_… or sk-…):".to_string(),
            246.0,
        ),
    };
    match run_modal(
        owner,
        is_dark,
        kind,
        title,
        &desc,
        "",
        existing_email,
        440.0,
        h,
    ) {
        Some(ModalOutcome::OpenCode(email, key)) => Some((email, key)),
        _ => None,
    }
}

/// Prompt to set or clear Claude Code's hourly spend target (`1..=10000` USD/h, blank = Off).
pub fn prompt_hourly_target(
    owner: HWND,
    is_dark: bool,
    current: Option<u32>,
) -> Option<Option<u32>> {
    let prefill = current.map(|v| v.to_string()).unwrap_or_default();
    match run_modal(
        owner,
        is_dark,
        ModalKind::HourlyTarget,
        "Hourly spend target",
        "Claude Code hourly spend target in whole USD (1–10000, leave blank for Off):",
        &prefill,
        None,
        420.0,
        170.0,
    ) {
        Some(ModalOutcome::HourlyTarget(v)) => Some(v),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_modal(
    owner: HWND,
    is_dark: bool,
    kind: ModalKind,
    title: &str,
    description: &str,
    initial_input1: &str,
    locked_email: Option<&str>,
    client_w_dip: f32,
    client_h_dip: f32,
) -> Option<ModalOutcome> {
    let _owner_guard = OwnerDisableGuard::new(owner);

    // SAFETY: Win32 modal window creation and local message loop on UI thread.
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            return None;
        };
        let class_name = w!("UsageDeckSettingsModal");
        MODAL_CLASS_REG.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(modal_wnd_proc),
                hInstance: hinstance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
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

        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
        let ex_style = WS_EX_DLGMODALFRAME;
        let mut wr = RECT {
            left: 0,
            top: 0,
            right: px(client_w_dip),
            bottom: px(client_h_dip),
        };
        let _ = AdjustWindowRectEx(&mut wr, style, false, ex_style);
        let win_w = wr.right - wr.left;
        let win_h = wr.bottom - wr.top;

        let mut owner_rc = RECT::default();
        let (x, y) = if !owner.is_invalid() && GetWindowRect(owner, &mut owner_rc).is_ok() {
            (
                owner_rc.left + ((owner_rc.right - owner_rc.left - win_w) / 2),
                owner_rc.top + ((owner_rc.bottom - owner_rc.top - win_h) / 2),
            )
        } else {
            (200, 200)
        };

        let title_w = wide(title);
        let Ok(hwnd) = CreateWindowExW(
            ex_style,
            class_name,
            PCWSTR(title_w.as_ptr()),
            style,
            x,
            y,
            win_w,
            win_h,
            Some(owner),
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return None;
        };

        apply_titlebar_theme(hwnd, is_dark);

        let font_norm = make_font(-px(13.5), 400);
        let font_sm = make_font(-px(12.0), 400);

        let mk = |ex: WINDOW_EX_STYLE,
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
            let wt = wide(text);
            let hmenu = if id == 0 {
                None
            } else {
                Some(HMENU(usize::from(id) as *mut std::ffi::c_void))
            };
            let ctrl = CreateWindowExW(
                ex,
                cls,
                PCWSTR(wt.as_ptr()),
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
        let lbl_st = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT);
        let lbl_ell = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT | SS_ENDELLIPSIS);
        let edit_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL);
        let pass_st = edit_st | WINDOW_STYLE(ES_PASSWORD);
        let chk_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX);
        let btn_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON);
        let def_st = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON);

        let pad = 20.0;
        let inner_w = client_w_dip - pad * 2.0;

        let _ = mk(
            no_ex,
            w!("STATIC"),
            description,
            lbl_st,
            pad,
            16.0,
            inner_w,
            34.0,
            0,
            font_norm,
        );

        let (edit1, edit2, chk_show, err_y) = match kind {
            ModalKind::Rename | ModalKind::HourlyTarget | ModalKind::SubscriptionEnd => {
                let e1 = mk(
                    no_ex,
                    w!("EDIT"),
                    initial_input1,
                    edit_st,
                    pad,
                    56.0,
                    inner_w,
                    28.0,
                    ID_INPUT_1,
                    font_norm,
                );
                (e1, HWND::default(), HWND::default(), 92.0)
            }
            ModalKind::ReplaceOpenCode => {
                let e1 = mk(
                    no_ex,
                    w!("EDIT"),
                    "",
                    pass_st,
                    pad,
                    56.0,
                    inner_w,
                    28.0,
                    ID_INPUT_1,
                    font_norm,
                );
                let chk = mk(
                    no_ex,
                    w!("BUTTON"),
                    "Show API key",
                    chk_st,
                    pad,
                    92.0,
                    140.0,
                    22.0,
                    ID_CHK_SHOW_KEY,
                    font_sm,
                );
                (e1, HWND::default(), chk, 122.0)
            }
            ModalKind::AddOpenCode => {
                let _ = mk(
                    no_ex,
                    w!("STATIC"),
                    "Email or label",
                    lbl_ell,
                    pad,
                    54.0,
                    inner_w,
                    18.0,
                    0,
                    font_sm,
                );
                let e1 = mk(
                    no_ex,
                    w!("EDIT"),
                    "",
                    edit_st,
                    pad,
                    74.0,
                    inner_w,
                    28.0,
                    ID_INPUT_1,
                    font_norm,
                );
                let _ = mk(
                    no_ex,
                    w!("STATIC"),
                    "API key",
                    lbl_ell,
                    pad,
                    110.0,
                    inner_w,
                    18.0,
                    0,
                    font_sm,
                );
                let e2 = mk(
                    no_ex,
                    w!("EDIT"),
                    "",
                    pass_st,
                    pad,
                    130.0,
                    inner_w,
                    28.0,
                    ID_INPUT_2,
                    font_norm,
                );
                let chk = mk(
                    no_ex,
                    w!("BUTTON"),
                    "Show API key",
                    chk_st,
                    pad,
                    164.0,
                    140.0,
                    22.0,
                    ID_CHK_SHOW_KEY,
                    font_sm,
                );
                (e1, e2, chk, 190.0)
            }
        };

        let lbl_error = mk(
            no_ex,
            w!("STATIC"),
            "",
            lbl_ell,
            pad,
            err_y,
            inner_w,
            20.0,
            ID_LBL_ERROR,
            font_sm,
        );

        let btn_y = client_h_dip - 46.0;
        let _ = mk(
            no_ex,
            w!("BUTTON"),
            "Save",
            def_st,
            client_w_dip - pad - 88.0 - 8.0 - 88.0,
            btn_y,
            88.0,
            30.0,
            ID_OK,
            font_norm,
        );
        let _ = mk(
            no_ex,
            w!("BUTTON"),
            "Cancel",
            btn_st,
            client_w_dip - pad - 88.0,
            btn_y,
            88.0,
            30.0,
            ID_CANCEL,
            font_norm,
        );

        let is_gem = crate::settings_dialog::theme::active_app_skin()
            == usagedeck_core::store::AppSkin::GeminiGem;
        let (bg_color, input_bg, fg_color, err_color) = match (is_dark, is_gem) {
            (true, true) => (
                colorref(0x16, 0x18, 0x1c),
                colorref(0x1e, 0x21, 0x27),
                colorref(0xf4, 0xf4, 0xf6),
                colorref(0xff, 0x80, 0x80),
            ),
            (true, false) => (
                colorref(0x20, 0x20, 0x20),
                colorref(0x2b, 0x2b, 0x2b),
                colorref(0xf4, 0xf4, 0xf5),
                colorref(0xff, 0x80, 0x80),
            ),
            (false, true) => (
                colorref(0xf4, 0xf6, 0xf8),
                colorref(0xff, 0xff, 0xff),
                colorref(0x16, 0x18, 0x1c),
                colorref(0xc1, 0x00, 0x07),
            ),
            (false, false) => (
                colorref(0xf3, 0xf3, 0xf3),
                colorref(0xff, 0xff, 0xff),
                colorref(0x18, 0x18, 0x1b),
                colorref(0xc1, 0x00, 0x07),
            ),
        };
        let bg_brush = CreateSolidBrush(bg_color);
        let input_brush = CreateSolidBrush(input_bg);

        ACTIVE_MODAL.with(|cell| {
            *cell.borrow_mut() = Some(ModalState {
                kind,
                locked_email: locked_email.map(str::to_string),
                is_dark,
                bg_color,
                input_bg,
                fg_color,
                err_color,
                bg_brush,
                input_brush,
                fonts: vec![font_norm, font_sm],
                edit1,
                edit2,
                chk_show,
                lbl_error,
                done: false,
                outcome: None,
            });
        });

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if !edit1.is_invalid() {
            let _ = SetFocus(Some(edit1));
            SendMessageW(edit1, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }

        let mut msg = MSG::default();
        loop {
            let done = ACTIVE_MODAL.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|b| b.as_ref().map(|s| s.done))
                    .unwrap_or(true)
            });
            if done {
                break;
            }
            let got = GetMessageW(&mut msg, None, 0, 0).0;
            if got <= 0 {
                break;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let _ = DestroyWindow(hwnd);
        ACTIVE_MODAL.with(|cell| cell.borrow_mut().take().and_then(|mut s| s.outcome.take()))
    }
}

fn handle_modal_command(id: u16, notify: u16) {
    if notify != BN_CLICKED {
        return;
    }
    ACTIVE_MODAL.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return;
        };
        let Some(state) = borrow.as_mut() else {
            return;
        };

        match id {
            ID_CANCEL => {
                state.outcome = None;
                state.done = true;
            }
            ID_CHK_SHOW_KEY => {
                // SAFETY: toggles password masking character on our own child Edit control.
                unsafe {
                    let checked = SendMessageW(state.chk_show, BM_GETCHECK, None, None).0 as usize
                        == BST_CHECKED;
                    let target = match state.kind {
                        ModalKind::AddOpenCode => state.edit2,
                        ModalKind::ReplaceOpenCode => state.edit1,
                        _ => HWND::default(),
                    };
                    if !target.is_invalid() {
                        let mask: usize = if checked { 0 } else { 0x25CF };
                        SendMessageW(target, EM_SETPASSWORDCHAR, Some(WPARAM(mask)), None);
                        let _ = InvalidateRect(Some(target), None, true);
                    }
                }
            }
            ID_OK => match state.kind {
                ModalKind::Rename => {
                    let trimmed = get_edit_text(state.edit1).trim().to_string();
                    if trimmed.encode_utf16().count() > MAX_NICKNAME_LENGTH {
                        set_ctrl_text(state.lbl_error, "Nickname must be 64 characters or fewer.");
                        return;
                    }
                    state.outcome = Some(ModalOutcome::Rename(trimmed));
                    state.done = true;
                }
                ModalKind::AddOpenCode => {
                    let email = get_edit_text(state.edit1).trim().to_string();
                    let key = get_edit_text(state.edit2).trim().to_string();
                    if email.is_empty() || email.encode_utf16().count() > MAX_EMAIL_LENGTH {
                        set_ctrl_text(
                            state.lbl_error,
                            "Enter an email or label (up to 320 characters).",
                        );
                        // SAFETY: focus invalid input.
                        unsafe {
                            let _ = SetFocus(Some(state.edit1));
                        }
                        return;
                    }
                    if !is_valid_opencode_key(&key) {
                        set_ctrl_text(
                            state.lbl_error,
                            "API key must start with oc_sk_ or sk- with no spaces.",
                        );
                        // SAFETY: focus invalid input.
                        unsafe {
                            let _ = SetFocus(Some(state.edit2));
                        }
                        return;
                    }
                    state.outcome = Some(ModalOutcome::OpenCode(email, key));
                    state.done = true;
                }
                ModalKind::ReplaceOpenCode => {
                    let key = get_edit_text(state.edit1).trim().to_string();
                    if !is_valid_opencode_key(&key) {
                        set_ctrl_text(
                            state.lbl_error,
                            "API key must start with oc_sk_ or sk- with no spaces.",
                        );
                        // SAFETY: focus invalid input.
                        unsafe {
                            let _ = SetFocus(Some(state.edit1));
                        }
                        return;
                    }
                    if let Some(email) = state.locked_email.clone() {
                        state.outcome = Some(ModalOutcome::OpenCode(email, key));
                        state.done = true;
                    }
                }
                ModalKind::HourlyTarget => {
                    let raw = get_edit_text(state.edit1).trim().to_string();
                    if raw.is_empty() {
                        state.outcome = Some(ModalOutcome::HourlyTarget(None));
                        state.done = true;
                        return;
                    }
                    let cleaned = raw.trim_start_matches('$').trim_end_matches("/h").trim();
                    match cleaned.parse::<f64>() {
                        Ok(n) if n.is_finite() && (1.0..=10_000.0).contains(&n) => {
                            state.outcome =
                                Some(ModalOutcome::HourlyTarget(Some(n.round() as u32)));
                            state.done = true;
                        }
                        _ => {
                            set_ctrl_text(
                                state.lbl_error,
                                "Enter a whole number between 1 and 10000, or leave blank.",
                            );
                            // SAFETY: focus invalid input.
                            unsafe {
                                let _ = SetFocus(Some(state.edit1));
                            }
                        }
                    }
                }
                ModalKind::SubscriptionEnd => {
                    let raw = get_edit_text(state.edit1).trim().to_string();
                    if raw.is_empty() {
                        state.outcome = Some(ModalOutcome::SubscriptionEnd(None));
                        state.done = true;
                        return;
                    }
                    match normalize_ymd_date(&raw) {
                        Some(ymd) => {
                            state.outcome = Some(ModalOutcome::SubscriptionEnd(Some(ymd)));
                            state.done = true;
                        }
                        None => {
                            set_ctrl_text(
                                state.lbl_error,
                                "Use YYYY-MM-DD format (e.g. 2026-12-31), or leave blank.",
                            );
                            // SAFETY: focus invalid input.
                            unsafe {
                                let _ = SetFocus(Some(state.edit1));
                            }
                        }
                    }
                }
            },
            _ => {}
        }
    });
}

unsafe extern "system" fn modal_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as u16;
            let notify = ((wparam.0 >> 16) & 0xffff) as u16;
            handle_modal_command(id, notify);
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as *mut std::ffi::c_void);
            let handled = ACTIVE_MODAL.with(|cell| {
                let Ok(borrow) = cell.try_borrow() else {
                    return false;
                };
                let Some(state) = borrow.as_ref() else {
                    return false;
                };
                let mut rc = RECT::default();
                if GetClientRect(hwnd, &mut rc).is_ok() {
                    FillRect(hdc, &rc, state.bg_brush);
                    return true;
                }
                false
            });
            if handled {
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let hdc = HDC(wparam.0 as *mut std::ffi::c_void);
            let ctrl = HWND(lparam.0 as *mut std::ffi::c_void);
            let res = ACTIVE_MODAL.with(|cell| {
                let Ok(borrow) = cell.try_borrow() else {
                    return None;
                };
                let state = borrow.as_ref()?;
                let fg = if ctrl == state.lbl_error {
                    state.err_color
                } else {
                    state.fg_color
                };
                SetTextColor(hdc, fg);
                SetBkMode(hdc, TRANSPARENT);
                SetBkColor(hdc, state.bg_color);
                Some(state.bg_brush.0 as isize)
            });
            match res {
                Some(hbr) => LRESULT(hbr),
                None => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as *mut std::ffi::c_void);
            let res = ACTIVE_MODAL.with(|cell| {
                let Ok(borrow) = cell.try_borrow() else {
                    return None;
                };
                let state = borrow.as_ref()?;
                if !state.is_dark {
                    return None;
                }
                SetTextColor(hdc, state.fg_color);
                SetBkColor(hdc, state.input_bg);
                SetBkMode(hdc, OPAQUE);
                Some(state.input_brush.0 as isize)
            });
            match res {
                Some(hbr) => LRESULT(hbr),
                None => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
        WM_CLOSE => {
            ACTIVE_MODAL.with(|cell| {
                if let Ok(mut borrow) = cell.try_borrow_mut() {
                    if let Some(state) = borrow.as_mut() {
                        state.outcome = None;
                        state.done = true;
                    }
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
