//! Win32 output. A failed paste is not reported as successful insertion.
pub use crate::clipboard::copy;
use std::{io, mem::size_of};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC_EX,
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
    if unsafe {
        SendInput(
            events.len() as u32,
            events.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    } != events.len() as u32
    {
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
        unsafe {
            SendInput(
                releases.len() as u32,
                releases.as_ptr(),
                size_of::<INPUT>() as i32,
            );
        }
        return Err(io::Error::other(
            "SendInput incomplete; target may be elevated (UIPI) or input blocked",
        ));
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
    send(&events)
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
                "release held modifiers",
            ));
        }
    }
    if mode != "keystrokes" {
        crate::clipboard::prepare_paste(hwnd, text)?;
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
            send(&events)?;
            events.clear();
        }
        let mut units = [0u16; 2];
        for &code in character.encode_utf16(&mut units).iter() {
            events.push(key(0, code, KEYEVENTF_UNICODE));
            events.push(key(0, code, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    if !events.is_empty() {
        send(&events)?;
    }
    Ok(())
}
