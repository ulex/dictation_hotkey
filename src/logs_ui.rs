//! Lazy, modeless log viewer. The UI receives only bounded snapshots while visible.
use crate::{diagnostics::Ring, output};
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
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static WINDOW: AtomicUsize = AtomicUsize::new(0);
const REFRESH: u32 = WM_APP + 20;
fn ring() -> &'static Mutex<Ring> {
    RING.get_or_init(|| Mutex::new(Ring::default()))
}
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
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
    SendMessageW(GetDlgItem(hwnd, 100), 0xB1, usize::MAX, -1); // EM_SETSEL: end
    SendMessageW(GetDlgItem(hwnd, 100), 0xB7, 0, 0); // EM_SCROLLCARET
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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
            let width = lp as u32 & 0xffff;
            let height = (lp as u32 >> 16) & 0xffff;
            MoveWindow(
                GetDlgItem(hwnd, 100),
                10,
                10,
                width.saturating_sub(20) as i32,
                height.saturating_sub(65) as i32,
                1,
            );
            MoveWindow(
                GetDlgItem(hwnd, 101),
                10,
                height.saturating_sub(45) as i32,
                110,
                30,
                1,
            );
            MoveWindow(
                GetDlgItem(hwnd, 102),
                135,
                height.saturating_sub(45) as i32,
                110,
                30,
                1,
            );
            0
        }
        WM_DESTROY => {
            WINDOW.store(0, Ordering::Relaxed);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
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
        let instance = GetModuleHandleW(null());
        let class = w("DictationHotkeyLogs");
        let mut wc: WNDCLASSW = zeroed();
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        wc.lpfnWndProc = Some(proc);
        wc.hbrBackground = (windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW + 1) as _;
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            w("Dictation Hotkey Logs (no speech content)").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
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
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w("EDIT").as_ptr(),
            w("").as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32,
            10,
            10,
            720,
            380,
            hwnd,
            100usize as _,
            instance,
            null(),
        );
        SendMessageW(edit, 0xC5, 300000, 0); // EM_LIMITTEXT
        for (id, text, x) in [(101, "Clear", 10), (102, "Copy Logs", 135)] {
            CreateWindowExW(
                0,
                w("BUTTON").as_ptr(),
                w(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                x,
                395,
                110,
                30,
                hwnd,
                id as usize as _,
                instance,
                null(),
            );
        }
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        render(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}
