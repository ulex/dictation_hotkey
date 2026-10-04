//! Shared native UI styling and GDI ownership. Light system colors also respect
//! Windows high-contrast settings; standard controls keep keyboard/accessibility support.
use crate::ui_font::Font;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND},
        Gdi::*,
    },
    UI::{
        Controls::{
            InitCommonControlsEx, SetWindowTheme, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES,
            INITCOMMONCONTROLSEX,
        },
        HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
        WindowsAndMessaging::*,
    },
};

pub fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub unsafe fn scale(hwnd: HWND, value: i32) -> i32 {
    (value as i64 * GetDpiForWindow(hwnd).max(96) as i64 / 96) as i32
}
pub unsafe fn high_contrast() -> bool {
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    let mut contrast: HIGHCONTRASTW = zeroed();
    contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
    SystemParametersInfoW(
        SPI_GETHIGHCONTRAST,
        contrast.cbSize,
        &mut contrast as *mut _ as _,
        0,
    ) != 0
        && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
}
pub unsafe fn init() {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_TAB_CLASSES,
    };
    InitCommonControlsEx(&controls);
}

pub struct Theme {
    body: Font,
    heading: Font,
    title: Font,
    mono: Font,
    mono_edit: bool,
}
impl Theme {
    fn new(dpi: u32, mono_edit: bool) -> Self {
        Self {
            body: Font::new(dpi, 15, 400, "Segoe UI"),
            heading: Font::new(dpi, 17, 600, "Segoe UI"),
            title: Font::new(dpi, 24, 600, "Segoe UI"),
            mono: Font::new(dpi, 14, 400, "Consolas"),
            mono_edit,
        }
    }
    fn font(&self, id: i32) -> HFONT {
        match id {
            900 => self.title.handle(),
            901..=909 => self.heading.handle(),
            100 if self.mono_edit => self.mono.handle(),
            _ => self.body.handle(),
        }
    }
    pub fn body_font(&self) -> HFONT {
        self.body.handle()
    }
}
// GWLP_USERDATA is reserved for this owned Theme in each of our UI windows.
pub unsafe fn theme(hwnd: HWND) -> Option<&'static Theme> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Theme).as_ref()
}
pub unsafe fn attach(hwnd: HWND, mono_edit: bool) {
    let theme = Box::new(Theme::new(GetDpiForWindow(hwnd).max(96), mono_edit));
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(theme) as isize);
    // Best effort: Windows 11 rounds the frame; Windows 10 simply ignores this.
    let corner = DWMWCP_ROUND;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
        &corner as *const _ as _,
        size_of::<i32>() as u32,
    );
}
pub unsafe fn detach(hwnd: HWND) {
    let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Theme;
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}
pub unsafe fn style_control(hwnd: HWND, child: HWND) {
    if let Some(theme) = theme(hwnd) {
        SendMessageW(
            child,
            WM_SETFONT,
            theme.font(GetDlgCtrlID(child)) as usize,
            1,
        );
    }
    SetWindowTheme(child, w("Explorer").as_ptr(), null());
}
unsafe extern "system" fn update_child(child: HWND, parent: LPARAM) -> i32 {
    style_control(parent as HWND, child);
    1
}
pub unsafe fn dpi_changed(hwnd: HWND, lp: LPARAM) {
    let mono_edit = theme(hwnd).is_some_and(|t| t.mono_edit);
    let new = Box::new(Theme::new(GetDpiForWindow(hwnd).max(96), mono_edit));
    let old = SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(new) as isize) as *mut Theme;
    EnumChildWindows(hwnd, Some(update_child), hwnd as LPARAM);
    // Controls have switched fonts before the old ones are released.
    if !old.is_null() {
        drop(Box::from_raw(old));
    }
    if lp != 0 {
        let rect = &*(lp as *const RECT);
        SetWindowPos(
            hwnd,
            null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    InvalidateRect(hwnd, null(), 1);
}

pub unsafe fn colors(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN) {
        return None;
    }
    let dc = wp as HDC;
    let child = lp as HWND;
    let muted = (910..=919).contains(&GetDlgCtrlID(child))
        || windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(child) == 0;
    SetTextColor(
        dc,
        GetSysColor(if muted {
            COLOR_GRAYTEXT
        } else {
            COLOR_WINDOWTEXT
        }),
    );
    SetBkColor(dc, GetSysColor(COLOR_WINDOW));
    Some(GetSysColorBrush(COLOR_WINDOW) as LRESULT)
}
pub unsafe fn center(hwnd: HWND) {
    let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    let mut rect: RECT = zeroed();
    if GetMonitorInfoW(monitor, &mut info) == 0 || GetWindowRect(hwnd, &mut rect) == 0 {
        return;
    }
    let work = info.rcWork;
    SetWindowPos(
        hwnd,
        null_mut(),
        work.left + ((work.right - work.left - (rect.right - rect.left)) / 2).max(0),
        work.top + ((work.bottom - work.top - (rect.bottom - rect.top)) / 2).max(0),
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}
pub unsafe fn client_size(hwnd: HWND, width: i32, height: i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: scale(hwnd, width),
        bottom: scale(hwnd, height),
    };
    AdjustWindowRectExForDpi(
        &mut rect,
        GetWindowLongW(hwnd, GWL_STYLE) as u32,
        0,
        GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
        GetDpiForWindow(hwnd).max(96),
    );
    SetWindowPos(
        hwnd,
        null_mut(),
        0,
        0,
        rect.right - rect.left,
        rect.bottom - rect.top,
        SWP_NOMOVE | SWP_NOZORDER,
    );
}
pub unsafe fn move_control(hwnd: HWND, id: i32, x: i32, y: i32, width: i32, height: i32) {
    MoveWindow(
        GetDlgItem(hwnd, id),
        scale(hwnd, x),
        scale(hwnd, y),
        scale(hwnd, width),
        scale(hwnd, height),
        1,
    );
}
pub unsafe fn paint_footer(hwnd: HWND, y: i32) {
    let mut paint: PAINTSTRUCT = zeroed();
    let dc = BeginPaint(hwnd, &mut paint);
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
    rect.top = y;
    rect.bottom = y + scale(hwnd, 1).max(1);
    FillRect(dc, &rect, GetSysColorBrush(COLOR_3DLIGHT));
    EndPaint(hwnd, &paint);
}
