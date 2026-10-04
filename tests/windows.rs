#![cfg(windows)]
//! Desktop integration tests. Run explicitly on an unlocked Windows desktop:
//! cargo test --test windows -- --ignored --test-threads=1
use dictation_hotkey_native::output;
use std::{
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
        Threading::{AttachThreadInput, GetCurrentThreadId},
    },
    UI::{
        Input::KeyboardAndMouse::{GetFocus, SetFocus, INPUT},
        WindowsAndMessaging::*,
    },
};
fn test_exe() -> std::ffi::OsString {
    std::env::var_os("DICTATION_TEST_EXE")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_dictation-hotkey-native").into())
}
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn pump() {
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
#[test]
#[ignore = "needs an unlocked interactive desktop and changes clipboard"]
fn unicode_clipboard_and_input_layout() {
    unsafe {
        assert_eq!(
            size_of::<INPUT>(),
            40,
            "Win64 INPUT must include the mouse union member"
        );
        unsafe extern "system" fn target_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        let class = w("DictationInputTestTarget");
        let wc = WNDCLASSW {
            lpszClassName: class.as_ptr(),
            lpfnWndProc: Some(target_proc),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let parent = Window(CreateWindowExW(
            0,
            class.as_ptr(),
            w("Dictation integration target").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            50,
            50,
            500,
            250,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        ));
        assert!(!parent.0.is_null());
        let edit = CreateWindowExW(
            0,
            w("EDIT").as_ptr(),
            w("").as_ptr(),
            WS_VISIBLE | WS_CHILD | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            10,
            10,
            450,
            180,
            parent.0,
            null_mut(),
            null_mut(),
            null(),
        );
        assert!(!edit.is_null());
        // A background cargo invocation may not own foreground activation rights.
        // Attach only while activating this controlled target, never in app output code.
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let attached = foreground != current && AttachThreadInput(current, foreground, 1) != 0;
        SetForegroundWindow(parent.0);
        SetFocus(edit);
        if attached {
            AttachThreadInput(current, foreground, 0);
        }
        pump();
        assert_eq!(
            GetForegroundWindow(),
            parent.0,
            "controlled target must be foreground before injecting input"
        );
        assert_eq!(GetFocus(), edit, "controlled EDIT must have keyboard focus");
        let text = "Dictation 😀 𝄞 café\r\n";
        output::copy(parent.0, text).unwrap();
        output::copy(parent.0, "").unwrap(); // an empty session must not replace/copy a prior result
        assert_ne!(OpenClipboard(parent.0), 0);
        let mem = GetClipboardData(13);
        assert!(!mem.is_null());
        let ptr = GlobalLock(mem) as *const u16;
        let expected: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        assert_eq!(std::slice::from_raw_parts(ptr, expected.len()), &expected);
        GlobalUnlock(mem);
        CloseClipboard();
        // Cross the keystroke batching boundary and include surrogate pairs.
        let text = "Hello 😀 𝄞 café ".repeat(17);
        for (mode, shortcut) in [
            ("keystrokes", "shift_insert"),
            ("paste", "shift_insert"),
            ("paste", "ctrl_v"),
        ] {
            SetWindowTextW(edit, w("").as_ptr());
            assert_eq!(GetForegroundWindow(), parent.0);
            assert_eq!(GetFocus(), edit);
            let sending = Instant::now();
            output::type_text(parent.0, &text, mode, shortcut).unwrap();
            eprintln!(
                "{mode}/{shortcut} SendInput elapsed: {:?}",
                sending.elapsed()
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut actual = vec![0u16; text.encode_utf16().count() + 1];
            loop {
                pump();
                let count = GetWindowTextW(edit, actual.as_mut_ptr(), actual.len() as i32);
                if String::from_utf16_lossy(&actual[..count as usize]) == text {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{mode}/{shortcut} did not reach controlled EDIT target; controlled value: {:?}",
                    String::from_utf16_lossy(&actual[..count as usize])
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
#[test]
#[ignore = "launches tray app/windows on the interactive desktop"]
fn app_shell_settings_logs_single_instance_and_shutdown() {
    assert!(
        unsafe { FindWindowW(w("DictationHotkeyNativeController").as_ptr(), null()) }.is_null(),
        "Close the existing native app before running this destructive shell test"
    );
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(std::process::Command::new(test_exe()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let controller;
    loop {
        let hwnd = unsafe { FindWindowW(w("DictationHotkeyNativeController").as_ptr(), null()) };
        if !hwnd.is_null() {
            controller = hwnd;
            break;
        }
        assert!(Instant::now() < deadline, "controller did not become ready");
        std::thread::sleep(Duration::from_millis(50));
    }
    let second = std::process::Command::new(test_exe()).status().unwrap();
    assert!(second.success());
    std::thread::sleep(Duration::from_millis(200));
    unsafe {
        let settings = FindWindowW(w("DictationHotkeyNativeSettings").as_ptr(), null());
        assert!(!settings.is_null(), "second instance should show Settings");
        assert!(!GetDlgItem(settings, 100).is_null(), "API key edit exists");
        assert_eq!(
            GetWindowLongPtrW(GetDlgItem(settings, 100), GWL_STYLE) & ES_PASSWORD as isize,
            ES_PASSWORD as isize
        );
        PostMessageW(settings, WM_CLOSE, 0, 0);
        let deadline = Instant::now() + Duration::from_secs(10);
        while IsWindow(settings) != 0 {
            assert!(Instant::now() < deadline, "Settings failed to close");
            std::thread::sleep(Duration::from_millis(50));
        }
        PostMessageW(controller, WM_COMMAND, 2, 0); // Logs
        let deadline = Instant::now() + Duration::from_secs(10);
        while FindWindowW(w("DictationHotkeyLogs").as_ptr(), null()).is_null() {
            assert!(Instant::now() < deadline, "Logs failed to open");
            std::thread::sleep(Duration::from_millis(50));
        }
        PostMessageW(controller, WM_COMMAND, 5, 0); // coordinated Quit
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.0.kill();
            panic!("app did not shut down");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
