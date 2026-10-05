//! Owned, DPI-scaled fonts. No font is deleted while its controls are alive.
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetStockObject, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DEFAULT_GUI_FONT, HFONT,
};

pub struct Font(HFONT);
impl Font {
    pub fn new(dpi: u32, pixels: i32, weight: i32, family: &str) -> Self {
        let name: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
        Self(unsafe {
            CreateFontW(
                -(pixels * dpi as i32 / 96),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                name.as_ptr(),
            )
        })
    }
    pub fn handle(&self) -> HFONT {
        if self.0.is_null() {
            unsafe { GetStockObject(DEFAULT_GUI_FONT) }
        } else {
            self.0
        }
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}
