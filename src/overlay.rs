//! Nonactivating, rounded native status popup; timers exist only while visible.
use crate::ui;
use std::{
    cell::Cell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};
pub const STOP_MESSAGE: u32 = WM_APP + 4;
thread_local! { static RECORDING: Cell<bool> = const { Cell::new(false) }; static PULSE: Cell<bool> = const { Cell::new(false) }; }
use ui::w;
unsafe fn round_region(hwnd: HWND) {
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    let radius = ui::scale(hwnd, 24);
    let region = CreateRoundRectRgn(0, 0, rect.right + 1, rect.bottom + 1, radius, radius);
    if !region.is_null() && SetWindowRgn(hwnd, region, 1) == 0 {
        DeleteObject(region);
    }
    // Windows owns a successfully assigned region.
}
unsafe fn position(hwnd: HWND) {
    let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return;
    }
    let work = info.rcWork;
    let width = ui::scale(hwnd, 560).min(work.right - work.left);
    let height = ui::scale(hwnd, 64);
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        work.left + (work.right - work.left - width) / 2,
        work.bottom - height - ui::scale(hwnd, 24),
        width,
        height,
        SWP_NOACTIVATE,
    );
    round_region(hwnd);
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
        WM_DPICHANGED => {
            ui::dpi_changed(hwnd, lp);
            position(hwnd);
            0
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            SetLayeredWindowAttributes(
                hwnd,
                0,
                if ui::high_contrast() { 255 } else { 248 },
                LWA_ALPHA,
            );
            position(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SIZE => {
            round_region(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            let contrast = ui::high_contrast();
            let brush = CreateSolidBrush(if contrast {
                GetSysColor(COLOR_WINDOW)
            } else {
                0x00282422
            });
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
            let font = ui::theme(hwnd)
                .map(|t| t.body_font())
                .unwrap_or_else(|| GetStockObject(DEFAULT_GUI_FONT));
            let previous = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(
                dc,
                if contrast {
                    GetSysColor(COLOR_WINDOWTEXT)
                } else {
                    0x00FAFAFA
                },
            );
            let mut text = [0u16; 512];
            let count = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            let recording = RECORDING.with(Cell::get);
            if recording {
                // Reuse the smoothed ball asset instead of a jagged GDI ellipse.
                let size = ui::scale(hwnd, if PULSE.with(Cell::get) { 18 } else { 16 });
                let icon = LoadImageW(
                    GetModuleHandleW(null()),
                    102usize as _,
                    IMAGE_ICON,
                    size,
                    size,
                    LR_DEFAULTCOLOR,
                );
                if !icon.is_null() {
                    DrawIconEx(
                        dc,
                        ui::scale(hwnd, 18),
                        (rect.bottom - size) / 2,
                        icon,
                        size,
                        size,
                        0,
                        null_mut(),
                        DI_NORMAL,
                    );
                    DestroyIcon(icon);
                }
                let mut stop_rect = rect;
                stop_rect.left = rect.right - ui::scale(hwnd, 92);
                stop_rect.right -= ui::scale(hwnd, 16);
                stop_rect.top = ui::scale(hwnd, 16);
                stop_rect.bottom -= ui::scale(hwnd, 16);
                let stop_brush = CreateSolidBrush(if contrast {
                    GetSysColor(COLOR_BTNFACE)
                } else {
                    0x00443E3A
                });
                let previous_brush = SelectObject(dc, stop_brush);
                let previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                RoundRect(
                    dc,
                    stop_rect.left,
                    stop_rect.top,
                    stop_rect.right,
                    stop_rect.bottom,
                    ui::scale(hwnd, 12),
                    ui::scale(hwnd, 12),
                );
                SelectObject(dc, previous_brush);
                SelectObject(dc, previous_pen);
                DeleteObject(stop_brush);
                if contrast {
                    SetTextColor(dc, GetSysColor(COLOR_BTNTEXT));
                }
                DrawTextW(
                    dc,
                    w("Stop").as_ptr(),
                    -1,
                    &mut stop_rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                if contrast {
                    SetTextColor(dc, GetSysColor(COLOR_WINDOWTEXT));
                }
                rect.left += ui::scale(hwnd, 52);
                rect.right -= ui::scale(hwnd, 108);
            } else {
                rect.left += ui::scale(hwnd, 24);
                rect.right -= ui::scale(hwnd, 24);
            }
            DrawTextW(
                dc,
                text.as_ptr(),
                count,
                &mut rect,
                DT_VCENTER
                    | DT_SINGLELINE
                    | DT_END_ELLIPSIS
                    | DT_NOPREFIX
                    | if recording { DT_LEFT } else { DT_CENTER },
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
        WM_NCDESTROY => {
            ui::detach(hwnd);
            DefWindowProcW(hwnd, msg, wp, lp)
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
                64,
                parent,
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err("create status overlay failed".into());
            }
            ui::attach(hwnd, false);
            SetLayeredWindowAttributes(
                hwnd,
                0,
                if ui::high_contrast() { 255 } else { 248 },
                LWA_ALPHA,
            );
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
