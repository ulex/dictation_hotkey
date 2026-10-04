//! DPI-scaled system UI font. Callers release it after all controls/paint operations stop using it.
use windows_sys::Win32::{Foundation::HWND, Graphics::Gdi::{CreateFontW, HFONT, DEFAULT_CHARSET, DEFAULT_QUALITY, FW_NORMAL}, UI::HiDpi::GetDpiForWindow};
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn create(hwnd: HWND) -> HFONT {
    let name: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
    unsafe { CreateFontW(-(15 * GetDpiForWindow(hwnd).max(96) / 96) as i32,0,0,0,FW_NORMAL as i32,0,0,0,
        DEFAULT_CHARSET as u32,0,0,DEFAULT_QUALITY as u32,0,name.as_ptr()) }
}
