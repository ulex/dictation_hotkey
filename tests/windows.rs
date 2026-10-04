#![cfg(windows)]
//! Desktop integration tests. Run explicitly on an unlocked Windows desktop:
//! cargo test --test windows -- --ignored --test-threads=1
use dictation_hotkey_native::{clipboard, output};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        Threading::{AttachThreadInput, GetCurrentThreadId},
    },
    UI::{
        Controls::TCM_GETITEMCOUNT,
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
            if msg == WM_TIMER && wp == clipboard::RESTORE_TIMER {
                clipboard::restore(hwnd).unwrap();
                return 0;
            }
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
        let original = "Keep this clipboard 😀";
        output::copy(parent.0, original).unwrap();
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
            // SendInput is asynchronous; let the controller-style restore timer fire.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                pump();
                if clipboard_text(parent.0) == original {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "original clipboard was not restored"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

unsafe fn clipboard_text(hwnd: HWND) -> String {
    assert_ne!(OpenClipboard(hwnd), 0);
    let mem = GetClipboardData(13);
    assert!(!mem.is_null());
    let ptr = GlobalLock(mem) as *const u16;
    assert!(!ptr.is_null());
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
    GlobalUnlock(mem);
    CloseClipboard();
    text
}

#[test]
#[ignore = "changes clipboard; run explicitly on an interactive desktop"]
fn clipboard_preserves_formats_empty_contents_and_newer_copies() {
    unsafe {
        let hwnd = Window(CreateWindowExW(
            0,
            w("STATIC").as_ptr(),
            w("").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        ));
        assert!(!hwnd.0.is_null());
        output::copy(hwnd.0, "original 😀").unwrap();
        let format = RegisterClipboardFormatW(w("DictationHotkeyTestFormat").as_ptr());
        assert_ne!(format, 0);
        let bytes = b"custom rich content\0";
        assert_ne!(OpenClipboard(hwnd.0), 0);
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        assert!(!mem.is_null());
        let ptr = GlobalLock(mem) as *mut u8;
        assert!(!ptr.is_null());
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        GlobalUnlock(mem);
        assert!(!SetClipboardData(format, mem).is_null());
        CloseClipboard();

        clipboard::prepare_paste(hwnd.0, "temporary").unwrap();
        assert_eq!(clipboard_text(hwnd.0), "temporary");
        assert_eq!(
            clipboard::prepare_paste(hwnd.0, "too soon")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        clipboard::restore(hwnd.0).unwrap();
        assert_eq!(clipboard_text(hwnd.0), "original 😀");
        assert_ne!(OpenClipboard(hwnd.0), 0);
        let restored = GetClipboardData(format);
        assert!(!restored.is_null());
        let ptr = GlobalLock(restored) as *const u8;
        assert_eq!(std::slice::from_raw_parts(ptr, bytes.len()), bytes);
        GlobalUnlock(restored);
        CloseClipboard();

        // A newer explicit Copy Last Text/user copy must not be overwritten.
        clipboard::prepare_paste(hwnd.0, "temporary").unwrap();
        output::copy(hwnd.0, "newer clipboard").unwrap();
        clipboard::restore(hwnd.0).unwrap();
        assert_eq!(clipboard_text(hwnd.0), "newer clipboard");

        assert_ne!(OpenClipboard(hwnd.0), 0);
        assert_ne!(EmptyClipboard(), 0);
        CloseClipboard();
        clipboard::prepare_paste(hwnd.0, "temporary").unwrap();
        clipboard::restore(hwnd.0).unwrap();
        assert_ne!(OpenClipboard(hwnd.0), 0);
        assert!(
            GetClipboardData(13).is_null(),
            "empty clipboard must stay empty"
        );
        CloseClipboard();
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
        let tabs = GetDlgItem(settings, 600);
        assert!(!tabs.is_null(), "native settings tabs exist");
        assert_eq!(SendMessageW(tabs, TCM_GETITEMCOUNT, 0, 0), 2);
        let body_font = SendMessageW(GetDlgItem(settings, 100), WM_GETFONT, 0, 0);
        let title_font = SendMessageW(GetDlgItem(settings, 900), WM_GETFONT, 0, 0);
        assert_ne!(body_font, 0, "DPI-scaled body font is assigned");
        assert_ne!(title_font, body_font, "title has distinct typography");
        assert_eq!(
            GetNextDlgTabItem(settings, GetDlgItem(settings, 100), 0),
            GetDlgItem(settings, 101),
            "Tab follows the visual field order"
        );
        assert_ne!(IsWindowVisible(GetDlgItem(settings, 100)), 0);
        assert_eq!(
            IsWindowVisible(GetDlgItem(settings, 103)),
            0,
            "connection fields start on the second page"
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
        let logs = FindWindowW(w("DictationHotkeyLogs").as_ptr(), null());
        assert!(!GetDlgItem(logs, 900).is_null(), "Logs has a title header");
        assert_ne!(SendMessageW(GetDlgItem(logs, 100), WM_GETFONT, 0, 0), 0);
        assert_ne!(
            GetWindowLongPtrW(GetDlgItem(logs, 100), GWL_STYLE) & ES_READONLY as isize,
            0
        );
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
