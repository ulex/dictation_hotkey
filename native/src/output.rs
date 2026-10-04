//! Win32 output. A failed paste is not reported as successful insertion.
use std::{io, mem::size_of, ptr::copy_nonoverlapping};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC_EX,
    },
};
pub const MARK: usize = 0xD1C7A710;
const CF_UNICODETEXT: u32 = 13;

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
// HWND is supplied by the controller's live UI window; clipboard APIs do not retain it.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy(hwnd: HWND, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let data: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "clipboard is in use",
            ));
        }
        let result = (|| {
            let mem = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2);
            if mem.is_null() {
                return Err(io::Error::last_os_error());
            }
            let dest = GlobalLock(mem) as *mut u16;
            if dest.is_null() {
                GlobalFree(mem);
                return Err(io::Error::last_os_error());
            }
            copy_nonoverlapping(data.as_ptr(), dest, data.len());
            GlobalUnlock(mem);
            if EmptyClipboard() == 0 {
                GlobalFree(mem);
                return Err(io::Error::last_os_error());
            }
            if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
                GlobalFree(mem);
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        CloseClipboard();
        result
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
        copy(hwnd, text)?;
        let keys: &[u16] = match shortcut {
            "ctrl_v" => &[0x11, 0x56],
            "ctrl_shift_v" => &[0x11, 0x10, 0x56],
            _ => &[0x10, 0x2d],
        };
        return combo(keys);
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
