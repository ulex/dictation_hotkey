//! Lazy, modeless log viewer. Only bounded operational snapshots reach the UI.
use crate::{diagnostics::Ring, output, ui};
use std::{
    mem::zeroed,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{InvalidateRect, COLOR_WINDOW},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETRECT, EM_SETSEL},
        WindowsAndMessaging::*,
    },
};
static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static WINDOW: AtomicUsize = AtomicUsize::new(0);
const REFRESH: u32 = WM_APP + 20;
use ui::w;
fn ring() -> &'static Mutex<Ring> {
    RING.get_or_init(|| Mutex::new(Ring::default()))
}
pub fn log(status: &str) {
    let seconds = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    ring()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(&format!("[+{seconds:.3}s] {status}"));
    let hwnd = WINDOW.load(Ordering::Relaxed) as HWND;
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, REFRESH, 0, 0);
        }
    }
}
unsafe fn render(hwnd: HWND) {
    let snapshot = ring().lock().unwrap_or_else(|e| e.into_inner()).snapshot();
    SetWindowTextW(GetDlgItem(hwnd, 100), w(&snapshot).as_ptr());
    SendMessageW(GetDlgItem(hwnd, 100), EM_SETSEL, usize::MAX, -1);
    SendMessageW(GetDlgItem(hwnd, 100), EM_SCROLLCARET, 0, 0);
}
unsafe fn layout(hwnd: HWND) {
    let s = |v| ui::scale(hwnd, v);
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right;
    let height = rect.bottom;
    let margin = s(24);
    MoveWindow(
        GetDlgItem(hwnd, 900),
        margin,
        s(18),
        (width - 2 * margin).max(1),
        s(34),
        1,
    );
    MoveWindow(
        GetDlgItem(hwnd, 910),
        margin,
        s(57),
        (width - 2 * margin).max(1),
        s(24),
        1,
    );
    let edit_width = (width - 2 * margin).max(1);
    let edit_height = (height - s(168)).max(1);
    let edit = GetDlgItem(hwnd, 100);
    MoveWindow(edit, margin, s(96), edit_width, edit_height, 1);
    let text_rect = RECT {
        left: s(12),
        top: s(10),
        right: edit_width - s(12),
        bottom: edit_height - s(10),
    };
    SendMessageW(edit, EM_SETRECT, 0, &text_rect as *const _ as isize);
    MoveWindow(
        GetDlgItem(hwnd, 101),
        margin,
        height - s(48),
        s(100),
        s(32),
        1,
    );
    MoveWindow(
        GetDlgItem(hwnd, 102),
        width - margin - s(116),
        height - s(48),
        s(116),
        s(32),
        1,
    );
    InvalidateRect(hwnd, null(), 1);
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = ui::colors(msg, wp, lp) {
        return result;
    }
    match msg {
        REFRESH => {
            render(hwnd);
            0
        }
        WM_COMMAND if wp & 0xffff == 101 => {
            ring().lock().unwrap_or_else(|e| e.into_inner()).clear();
            render(hwnd);
            0
        }
        WM_COMMAND if wp & 0xffff == 102 => {
            let snapshot = ring().lock().unwrap_or_else(|e| e.into_inner()).snapshot();
            let _ = output::copy(hwnd, &snapshot);
            0
        }
        WM_SIZE => {
            layout(hwnd);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            info.ptMinTrackSize.x = ui::scale(hwnd, 560);
            info.ptMinTrackSize.y = ui::scale(hwnd, 360);
            0
        }
        WM_DPICHANGED => {
            ui::dpi_changed(hwnd, lp);
            layout(hwnd);
            0
        }
        WM_PAINT => {
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            ui::paint_footer(hwnd, rect.bottom - ui::scale(hwnd, 64));
            0
        }
        WM_DESTROY => {
            WINDOW.store(0, Ordering::Relaxed);
            0
        }
        WM_NCDESTROY => {
            ui::detach(hwnd);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
/// Route modeless dialog keyboard navigation from either application message loop.
/// # Safety
/// The message and window handles must be live on the owning UI thread.
pub unsafe fn dialog_message(msg: &MSG) -> bool {
    let hwnd = WINDOW.load(Ordering::Relaxed) as HWND;
    if hwnd.is_null() || (msg.hwnd != hwnd && IsChild(hwnd, msg.hwnd) == 0) {
        return false;
    }
    if msg.message == WM_KEYDOWN && msg.wParam == 0x1b {
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
        return true;
    }
    IsDialogMessageW(hwnd, msg) != 0
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn open(parent: HWND) {
    unsafe {
        let existing = WINDOW.load(Ordering::Relaxed) as HWND;
        if !existing.is_null() {
            ShowWindow(existing, SW_SHOW);
            SetForegroundWindow(existing);
            return;
        }
        ui::init();
        let instance = GetModuleHandleW(null());
        let class = w("DictationHotkeyLogs");
        let mut wc: WNDCLASSW = zeroed();
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        wc.lpfnWndProc = Some(proc);
        wc.hbrBackground = (COLOR_WINDOW + 1) as _;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            w("Dictation Hotkey — Activity logs").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            500,
            parent,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        ui::attach(hwnd, true);
        let add = |kind: &str, text: &str, style: u32, id: usize| {
            let child = CreateWindowExW(
                0,
                w(kind).as_ptr(),
                w(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                1,
                1,
                hwnd,
                id as _,
                instance,
                null(),
            );
            ui::style_control(hwnd, child);
            child
        };
        add("STATIC", "Activity logs", 0, 900);
        add(
            "STATIC",
            "Operational events only. Recorded audio and transcripts are not shown.",
            0,
            910,
        );
        let edit = add(
            "EDIT",
            "",
            WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
            100,
        );
        SendMessageW(edit, EM_SETLIMITTEXT, 300000, 0);
        add("BUTTON", "&Clear", WS_TABSTOP, 101);
        add("BUTTON", "&Copy logs", WS_TABSTOP, 102);
        ui::client_size(hwnd, 760, 500);
        layout(hwnd);
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        render(hwnd);
        ui::center(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}
