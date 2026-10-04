#![cfg(windows)]
//! Opt-in shell-level Win+H regression. No microphone/key needed; no settings saved.
use dictation_hotkey_native::{config::Config, output, paths};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        },
        WindowsAndMessaging::*,
    },
};
fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
unsafe fn window(class: &str) -> HWND {
    FindWindowW(w(class).as_ptr(), null())
}
unsafe fn pump() {
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
fn wait_for(description: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        unsafe {
            pump();
        }
        assert!(Instant::now() < deadline, "timed out: {description}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
unsafe fn inject(keys: &[(u16, bool)]) {
    let events: Vec<_> = keys
        .iter()
        .map(|&(vk, down)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if down { 0 } else { KEYEVENTF_KEYUP }
                        | if matches!(vk, 0x5b | 0x5c) {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            0
                        },
                    // Simulate external input, not the app's own tagged output.
                    dwExtraInfo: output::MARK ^ 1,
                    ..Default::default()
                },
            },
        })
        .collect();
    assert_eq!(
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            size_of::<INPUT>() as i32
        ),
        events.len() as u32
    );
}
#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(hwnd: HWND, attribute: u32, value: *mut u32, size: u32) -> i32;
}
unsafe fn start_visible() -> bool {
    let start = FindWindowW(
        w("Windows.UI.Core.CoreWindow").as_ptr(),
        w("Start").as_ptr(),
    );
    // UWP shell windows can retain WS_VISIBLE while DWM-cloaked.
    let mut cloaked = 0u32;
    !start.is_null()
        && IsWindowVisible(start) != 0
        && DwmGetWindowAttribute(start, 14, &mut cloaked, size_of::<u32>() as u32) >= 0
        && cloaked == 0
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Target(HWND);
impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
unsafe fn focus(hwnd: HWND) {
    wait_for(
        "controlled target foreground (use a foreground terminal)",
        || {
            let current = GetCurrentThreadId();
            let foreground = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
            let attached = foreground != 0
                && foreground != current
                && AttachThreadInput(current, foreground, 1) != 0;
            SetForegroundWindow(hwnd);
            SetFocus(GetDlgItem(hwnd, 1));
            if attached {
                AttachThreadInput(current, foreground, 0);
            }
            pump();
            GetForegroundWindow() == hwnd
        },
    );
}
#[test]
#[ignore = "requires foreground unlocked English Windows desktop, no running app, Win+H enabled and empty API key; opens Start as a control"]
fn win_h_triggers_app_without_start_menu_or_leaked_h() {
    let config = Config::load(&paths::config().unwrap()).unwrap();
    assert!(
        config.boolean("hotkey_win_h") && config.string("api_key").is_empty(),
        "use an unconfigured profile; this probe must not start real recording"
    );
    unsafe {
        assert!(
            !GetForegroundWindow().is_null(),
            "reconnect/unlock the Windows desktop before running this test"
        );
        assert!(
            window("DictationHotkeyNativeController").is_null(),
            "close the app first"
        );
        assert!(!start_visible(), "close Start first");
        for vk in [0x10, 0x11, 0x12, 0x5b, 0x5c] {
            assert!(GetAsyncKeyState(vk) >= 0, "release held modifiers");
        }
        let exe = std::env::var_os("DICTATION_TEST_EXE")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_dictation-hotkey-native").into());
        let mut child = Child(std::process::Command::new(exe).spawn().unwrap());
        wait_for("first-run Settings", || {
            !window("DictationHotkeyNativeSettings").is_null()
        });
        let controller = window("DictationHotkeyNativeController");
        assert!(!controller.is_null());
        PostMessageW(window("DictationHotkeyNativeSettings"), WM_CLOSE, 0, 0);
        wait_for("Settings closed", || {
            window("DictationHotkeyNativeSettings").is_null()
        });
        unsafe extern "system" fn target_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        let class = w("DictationHotkeyTestTarget");
        let wc = WNDCLASSW {
            lpszClassName: class.as_ptr(),
            lpfnWndProc: Some(target_proc),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let target = Target(CreateWindowExW(
            0,
            class.as_ptr(),
            w("Hotkey regression target").as_ptr(),
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
        assert!(!target.0.is_null());
        let edit = CreateWindowExW(
            0,
            w("EDIT").as_ptr(),
            w("").as_ptr(),
            WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            10,
            10,
            450,
            180,
            target.0,
            1usize as _,
            null_mut(),
            null(),
        );
        assert!(!edit.is_null());
        focus(target.0);
        // Positive control: ensure the probe can actually observe this shell's Start.
        inject(&[(0x5b, true), (0x5b, false)]);
        wait_for("bare Win opens Start (positive control)", || {
            start_visible()
        });
        inject(&[(0x1b, true), (0x1b, false)]);
        wait_for("Start dismissed", || !start_visible());
        for win in [0x5b, 0x5c] {
            for win_first in [false, true] {
                focus(target.0);
                let release = if win_first {
                    [(win, false), (0x48, false)]
                } else {
                    [(0x48, false), (win, false)]
                };
                inject(&[(win, true), (0x48, true), release[0], release[1]]);
                // Missing-key handling opens Settings: this proves a real hook action,
                // not merely that native dictation happened to stay hidden.
                wait_for("Win+H opens app Settings", || {
                    !window("DictationHotkeyNativeSettings").is_null()
                });
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline {
                    pump();
                    assert!(!start_visible(), "intercepted Win+H opened Start");
                    std::thread::sleep(Duration::from_millis(10));
                }
                let mut text = [0u16; 16];
                assert_eq!(
                    GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32),
                    0,
                    "H leaked into target"
                );
                assert!(GetAsyncKeyState(win as i32) >= 0, "Win key left stuck");
                PostMessageW(window("DictationHotkeyNativeSettings"), WM_CLOSE, 0, 0);
                wait_for("Settings closed", || {
                    window("DictationHotkeyNativeSettings").is_null()
                });
            }
        }
        // Interception must not permanently disable the normal Start shortcut.
        focus(target.0);
        inject(&[(0x5b, true), (0x5b, false)]);
        wait_for("bare Win still opens Start", || start_visible());
        inject(&[(0x1b, true), (0x1b, false)]);
        wait_for("Start dismissed", || !start_visible());
        PostMessageW(controller, WM_COMMAND, 5, 0);
        wait_for("app shutdown", || child.0.try_wait().unwrap().is_some());
        assert!(child.0.wait().unwrap().success());
    }
}
