//! Narrow C ABI to Apple frameworks. Swift owns Cocoa objects; Rust owns sessions.
use std::{
    ffi::{c_char, c_void, CString},
    io,
};
pub(crate) type Action = extern "C" fn(*mut c_void, i32, *const c_char);
unsafe extern "C" {
    pub fn dh_run(context: *mut c_void, action: Action, config: *const c_char);
    pub fn dh_ui_status(text: *const c_char, busy: i32);
    pub fn dh_ui_error(text: *const c_char);
    pub fn dh_ui_config(config: *const c_char) -> i32;
    pub fn dh_output(text: *const c_char, mode: i32) -> i32;
    pub fn dh_can_insert() -> i32;
    pub fn dh_capture(
        context: *mut c_void,
        stopped: extern "C" fn(*mut c_void) -> i32,
        pcm: extern "C" fn(*mut c_void, *const u8, usize) -> i32,
    ) -> i32;
    pub fn dh_ws_new(url: *const c_char, key: *const c_char) -> *mut c_void;
    pub fn dh_ws_send(handle: *mut c_void, text: *const c_char) -> i32;
    pub fn dh_ws_receive(handle: *mut c_void, bytes: *mut u8, capacity: usize) -> isize;
    pub fn dh_http_new(
        path: *const c_char,
        boundary: *const c_char,
        key: *const c_char,
    ) -> *mut c_void;
    pub fn dh_http_receive(handle: *mut c_void, bytes: *mut u8, capacity: usize) -> isize;
    pub fn dh_net_close(handle: *mut c_void);
    pub fn dh_net_free(handle: *mut c_void);
}
pub(crate) fn string(text: &str) -> io::Result<CString> {
    CString::new(text).map_err(|_| io::Error::other("text contains a null character"))
}
