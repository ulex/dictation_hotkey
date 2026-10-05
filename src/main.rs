#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
#[path = "platform/windows/app.rs"]
mod app;

#[cfg(windows)]
fn main() {
    if let Err(error) = app::run() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Dictation Hotkey".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = dictation_hotkey_native::macos_app::run() {
        eprintln!("Dictation Hotkey: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("Dictation Hotkey supports Windows and macOS; this platform can run core tests.");
    std::process::exit(1);
}
