//! Win32 output. A failed paste is not reported as successful insertion.
pub use crate::clipboard::copy;
use std::{io, mem::size_of};
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND},
    System::DataExchange::GetOpenClipboardWindow,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD,
            KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
            MAPVK_VK_TO_VSC_EX,
        },
        WindowsAndMessaging::{
            GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        },
    },
};
pub const MARK: usize = 0xD1C7A710;

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MARK,
            },
        },
    }
}
fn send(events: &[INPUT]) -> io::Result<()> {
    send_with(events, |events| unsafe {
        // SendInput can fail without setting last-error (including UIPI). Clear stale
        // errors and capture the result before cleanup or any other Win32 call.
        SetLastError(0);
        let sent = SendInput(
            events.len() as u32,
            events.as_ptr(),
            size_of::<INPUT>() as i32,
        );
        (sent, GetLastError())
    })
}

fn send_with(events: &[INPUT], mut inject: impl FnMut(&[INPUT]) -> (u32, u32)) -> io::Result<()> {
    let (sent, error) = inject(events);
    if sent != events.len() as u32 {
        // A partial batch can leave a modifier down. Release every key in reverse order;
        // do not retry the text (it may already be partly inserted).
        let releases: Vec<INPUT> = events
            .iter()
            .rev()
            .map(|event| {
                let mut release = *event;
                unsafe {
                    release.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;
                }
                release
            })
            .collect();
        let (released, release_error) = inject(&releases);
        return Err(io::Error::other(format!(
            "SendInput incomplete: inserted {sent}/{} events; Win32 error: {error}; key-up cleanup: {released}/{} events, Win32 error: {release_error}; text not retried (UIPI is not distinguishable from these results)",
            events.len(), releases.len(),
        )));
    }
    Ok(())
}
/// Called only for a Win-up following an intercepted reserved shortcut. Windows
/// otherwise sees Win-down/Win-up without H/C/F23 and opens Start. VK 0xE8 is
/// unassigned: it masks the shell's menu activation without typing text or
/// disturbing a user's held Ctrl/Alt/Shift keys. All three events are tagged so
/// our hook passes them through without recursion or changing physical state.
pub fn mask_windows_release(vk: u16, scan: u16) -> io::Result<()> {
    if !matches!(vk, 0x5b | 0x5c) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a Win key"));
    }
    send(&[
        key(0xe8, 0, 0),
        key(0xe8, 0, KEYEVENTF_KEYUP),
        key(vk, scan, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
    ])
}

fn context_error(stage: &str, error: io::Error) -> io::Error {
    // Preserve WouldBlock so clipboard contention and held modifiers remain retryable.
    io::Error::new(error.kind(), format!("{stage}: {error}"))
}

/// Failure-only snapshot. Never inspect window titles, text, clipboard contents,
/// or process paths. This is diagnostic context, not proof of the failure's cause.
pub fn failure_context() -> String {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut pid = 0;
        let tid = GetWindowThreadProcessId(foreground, &mut pid);
        let mut gui = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        SetLastError(0);
        let gui_ok = GetGUIThreadInfo(tid, &mut gui) != 0;
        let gui_error = GetLastError();
        let clipboard_window = GetOpenClipboardWindow();
        let mut clipboard_pid = 0;
        if !clipboard_window.is_null() {
            GetWindowThreadProcessId(clipboard_window, &mut clipboard_pid);
        }
        let held: Vec<_> = [0x10, 0x11, 0x12, 0x5b, 0x5c]
            .into_iter()
            .filter(|&vk| GetAsyncKeyState(vk) < 0)
            .collect();
        format!(
            "foreground: {foreground:p}, pid: {pid}, tid: {tid}; GUI query: {gui_ok}, Win32 error: {gui_error}, focus: {:p}, active: {:p}, flags: 0x{:x}; held modifier VKs: {held:x?}; clipboard open window: {clipboard_window:p}, pid: {clipboard_pid}",
            gui.hwndFocus, gui.hwndActive, gui.flags,
        )
    }
}

fn combo(keys: &[u16]) -> io::Result<()> {
    let mut events = Vec::with_capacity(keys.len() * 2);
    for &vk in keys {
        let mapped = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
        let ext = if vk == 0x2d || mapped >> 8 == 0xe0 {
            KEYEVENTF_EXTENDEDKEY
        } else {
            0
        };
        events.push(key(vk, mapped as u16 & 0xff, ext));
    }
    for mut event in events.clone().into_iter().rev() {
        unsafe {
            event.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;
        }
        events.push(event);
    }
    send(&events).map_err(|e| context_error("paste shortcut injection", e))
}
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn type_text(hwnd: HWND, text: &str, mode: &str, shortcut: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    for vk in [0x10, 0x11, 0x12, 0x5b, 0x5c] {
        if unsafe { GetAsyncKeyState(vk) } < 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("release held modifier (virtual key: 0x{vk:02x})"),
            ));
        }
    }
    if mode != "keystrokes" {
        crate::clipboard::prepare_paste(hwnd, text)
            .map_err(|e| context_error("clipboard paste preparation (no input sent)", e))?;
        let keys: &[u16] = match shortcut {
            "ctrl_v" => &[0x11, 0x56],
            "ctrl_shift_v" => &[0x11, 0x10, 0x56],
            _ => &[0x10, 0x2d],
        };
        let result = combo(keys);
        if result.is_err() {
            let _ = crate::clipboard::restore(hwnd);
        }
        return result;
    }
    let mut events = Vec::with_capacity(500);
    for character in text.chars() {
        if events.len() + character.len_utf16() * 2 > 500 {
            send(&events).map_err(|e| context_error("Unicode text injection", e))?;
            events.clear();
        }
        let mut units = [0u16; 2];
        for &code in character.encode_utf16(&mut units).iter() {
            events.push(key(0, code, KEYEVENTF_UNICODE));
            events.push(key(0, code, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    if !events.is_empty() {
        send(&events).map_err(|e| context_error("Unicode text injection", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_success_does_not_run_cleanup() {
        let events = [key(0x11, 0, 0)];
        let mut calls = 0;
        send_with(&events, |batch| {
            calls += 1;
            (batch.len() as u32, 0)
        })
        .unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn failed_send_preserves_original_error_and_never_retries_text() {
        let events = [
            key(0, 0x1234, KEYEVENTF_UNICODE),
            key(0, 0x1234, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        ];
        for sent in [0, 1] {
            let mut calls = 0;
            let error = send_with(&events, |batch| {
                calls += 1;
                if calls == 1 {
                    (sent, 5)
                } else {
                    assert_eq!(calls, 2, "only one cleanup attempt, no text retry");
                    for event in batch {
                        unsafe {
                            assert_ne!(event.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
                            assert_eq!(event.Anonymous.ki.dwExtraInfo, MARK);
                        }
                    }
                    (0, 87)
                }
            })
            .unwrap_err()
            .to_string();
            assert_eq!(calls, 2);
            assert!(error.contains(&format!("inserted {sent}/2 events; Win32 error: 5")));
            assert!(error.contains("key-up cleanup: 0/2 events, Win32 error: 87"));
            assert!(!error.contains("1234"), "do not log injected text");
        }
    }

    #[test]
    fn failure_without_win32_error_is_still_reported() {
        let error = send_with(&[key(0x11, 0, 0)], |_| (0, 0)).unwrap_err();
        assert!(error
            .to_string()
            .contains("inserted 0/1 events; Win32 error: 0"));
    }

    #[test]
    fn stage_context_preserves_retryable_errors() {
        let error = context_error(
            "clipboard paste preparation (no input sent)",
            io::Error::new(io::ErrorKind::WouldBlock, "clipboard is in use"),
        );
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            error.to_string(),
            "clipboard paste preparation (no input sent): clipboard is in use"
        );
    }
}
