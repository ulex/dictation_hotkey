//! Nonactivating, layered status popup; timers exist only while visible.
use std::{
    cell::Cell,
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};
pub const STOP_MESSAGE: u32 = WM_APP + 4;
thread_local! { static RECORDING: Cell<bool> = const { Cell::new(false) }; static PULSE: Cell<bool> = const { Cell::new(false) }; }
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn position(hwnd: HWND) {
    let mut work: RECT = zeroed();
    if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as _, 0) == 0 {
        return;
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    let width = (560 * dpi / 96) as i32;
    let height = (56 * dpi / 96) as i32;
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        work.left + (work.right - work.left - width) / 2,
        work.bottom - height - (28 * dpi / 96) as i32,
        width,
        height,
        SWP_NOACTIVATE,
    );
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP => {
            PostMessageW(GetWindow(hwnd, GW_OWNER), STOP_MESSAGE, 0, 0);
            0
        }
        WM_TIMER if wp == 1 => {
            PULSE.with(|p| p.set(!p.get()));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 2 => {
            ShowWindow(hwnd, SW_HIDE);
            KillTimer(hwnd, 2);
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
            position(hwnd);
            0
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            let brush = CreateSolidBrush(0x00262626);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let previous = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00FFFFFF);
            let mut text = [0u16; 512];
            let count = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            let recording = RECORDING.with(Cell::get);
            if recording {
                let size = (18 * GetDpiForWindow(hwnd).max(96) / 96) as i32;
                let brush = CreateSolidBrush(if PULSE.with(Cell::get) {
                    0x004848EE
                } else {
                    0x006E6EEE
                });
                let previous_brush = SelectObject(dc, brush);
                Ellipse(
                    dc,
                    12,
                    (rect.bottom - size) / 2,
                    12 + size,
                    (rect.bottom + size) / 2,
                );
                SelectObject(dc, previous_brush);
                DeleteObject(brush);
                rect.left += 36;
            }
            rect.left += 10;
            rect.right -= 10;
            DrawTextW(
                dc,
                text.as_ptr(),
                count,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
            EndPaint(hwnd, &paint);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 2);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
pub struct Overlay {
    hwnd: HWND,
}
impl Overlay {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn create(parent: HWND) -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = w("DictationHotkeyOverlay");
            let mut wc: WNDCLASSW = zeroed();
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            wc.lpfnWndProc = Some(proc);
            wc.hCursor = LoadCursorW(null_mut(), IDC_HAND);
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
                class.as_ptr(),
                w("").as_ptr(),
                WS_POPUP,
                0,
                0,
                560,
                56,
                parent,
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err("create status overlay failed".into());
            }
            SetLayeredWindowAttributes(hwnd, 0, 242, LWA_ALPHA);
            position(hwnd);
            Ok(Self { hwnd })
        }
    }
    pub fn show(&self, message: &str, recording: bool, persistent: bool) {
        unsafe {
            KillTimer(self.hwnd, 1);
            KillTimer(self.hwnd, 2);
            RECORDING.with(|r| r.set(recording));
            SetWindowTextW(self.hwnd, w(message).as_ptr());
            position(self.hwnd);
            InvalidateRect(self.hwnd, null(), 0);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            if recording {
                SetTimer(self.hwnd, 1, 450, None);
            } else if !persistent {
                SetTimer(self.hwnd, 2, 3000, None);
            }
        }
    }
    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
            KillTimer(self.hwnd, 1);
            KillTimer(self.hwnd, 2);
        }
    }
}
impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
