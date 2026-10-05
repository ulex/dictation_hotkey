//! UI-thread clipboard transactions. Never overwrite a newer clipboard owner.
use std::{cell::RefCell, io, mem::size_of, ptr::copy_nonoverlapping};
use windows_sys::Win32::{
    Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HWND},
    Graphics::Gdi::{
        DeleteEnhMetaFile, DeleteMetaFile, DeleteObject, GetEnhMetaFileBits, GetMetaFileBitsEx,
        GetObjectW, BITMAP,
    },
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardOwner, GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
            METAFILEPICT,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        Ole::OleDuplicateData,
    },
    UI::WindowsAndMessaging::{KillTimer, SetTimer},
};

pub const RESTORE_TIMER: usize = 2;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FORMATS: usize = 256;
const UNICODE_TEXT: u32 = 13;

struct Open;
impl Open {
    fn new(hwnd: HWND) -> io::Result<Self> {
        if unsafe { OpenClipboard(hwnd) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "clipboard is in use",
            ));
        }
        Ok(Self)
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
struct Format {
    id: u32,
    data: HANDLE,
}
impl Drop for Format {
    fn drop(&mut self) {
        if self.data.is_null() {
            return;
        }
        unsafe {
            match self.id {
                2 | 9 | 0x82 => {
                    DeleteObject(self.data);
                }
                14 | 0x8e => {
                    DeleteEnhMetaFile(self.data);
                }
                3 | 0x83 => {
                    let pict = GlobalLock(self.data) as *const METAFILEPICT;
                    if !pict.is_null() {
                        DeleteMetaFile((*pict).hMF);
                        GlobalUnlock(self.data);
                    }
                    GlobalFree(self.data);
                }
                _ => {
                    GlobalFree(self.data);
                }
            }
        }
    }
}
struct Pending {
    formats: Vec<Format>,
    sequence: u32,
    paste_data: HANDLE,
    emptied: bool,
}
thread_local! { static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) }; }

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored,
    Superseded,
    Idle,
}

// Synthesized ANSI/OEM text can advance the sequence without replacing our
// Unicode allocation. Ownership + handle identity distinguish that from a copy.
fn still_ours(
    partial: bool,
    has_paste_data: bool,
    same_sequence: bool,
    same_owner: bool,
    same_data: bool,
) -> bool {
    if partial || !has_paste_data {
        same_sequence
    } else {
        same_owner && same_data
    }
}

// Clipboard must stay open throughout snapshot and replacement, closing the race
// with other applications. Delayed-rendered formats are materialized by GetClipboardData.
unsafe fn snapshot() -> io::Result<Vec<Format>> {
    let mut formats = Vec::new();
    let mut id = 0;
    let mut bytes = 0usize;
    loop {
        SetLastError(0);
        id = EnumClipboardFormats(id);
        if id == 0 {
            if GetLastError() != 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(formats);
        }
        if formats.len() == MAX_FORMATS || (0x200..=0x3ff).contains(&id) {
            return Err(io::Error::other(
                "clipboard cannot be safely preserved; use keystroke output",
            ));
        }
        let data = GetClipboardData(id);
        if data.is_null() {
            return Err(io::Error::other("cannot read original clipboard format"));
        }
        let size = if matches!(id, 2 | 0x82) {
            let mut bitmap: BITMAP = std::mem::zeroed();
            if GetObjectW(data, size_of::<BITMAP>() as i32, &mut bitmap as *mut _ as _) == 0 {
                return Err(io::Error::last_os_error());
            }
            (bitmap.bmWidthBytes as usize).saturating_mul(bitmap.bmHeight.unsigned_abs() as usize)
        } else if matches!(id, 14 | 0x8e) {
            GetEnhMetaFileBits(data, 0, std::ptr::null_mut()) as usize
        } else if matches!(id, 3 | 0x83) {
            let pict = GlobalLock(data) as *const METAFILEPICT;
            if pict.is_null() {
                return Err(io::Error::last_os_error());
            }
            let size = GetMetaFileBitsEx((*pict).hMF, 0, std::ptr::null_mut()) as usize;
            GlobalUnlock(data);
            size
        } else if id == 9 {
            2048 // maximum logical palette size
        } else {
            GlobalSize(data)
        };
        bytes = bytes.saturating_add(size);
        if bytes > MAX_BYTES {
            return Err(io::Error::other(
                "clipboard too large to preserve; use keystroke output",
            ));
        }
        // Display variants use the same handle types as their normal formats.
        let duplicate_format = match id {
            0x82 => 2,
            0x83 => 3,
            0x8e => 14,
            _ => id,
        };
        let duplicate = OleDuplicateData(data, duplicate_format as u16, GMEM_MOVEABLE);
        if duplicate.is_null() {
            return Err(io::Error::other(
                "cannot preserve original clipboard format",
            ));
        }
        formats.push(Format {
            id,
            data: duplicate,
        });
    }
}

unsafe fn replace(text: &str) -> io::Result<()> {
    let data: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut format = Format {
        id: UNICODE_TEXT,
        data: GlobalAlloc(GMEM_MOVEABLE, data.len() * 2),
    };
    if format.data.is_null() {
        return Err(io::Error::last_os_error());
    }
    let dest = GlobalLock(format.data) as *mut u16;
    if dest.is_null() {
        return Err(io::Error::last_os_error());
    }
    copy_nonoverlapping(data.as_ptr(), dest, data.len());
    GlobalUnlock(format.data);
    if EmptyClipboard() == 0 || SetClipboardData(UNICODE_TEXT, format.data).is_null() {
        return Err(io::Error::last_os_error());
    }
    format.data = std::ptr::null_mut(); // Windows now owns the allocation.
    Ok(())
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy(hwnd: HWND, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let _open = Open::new(hwnd)?;
    unsafe {
        replace(text)?;
    }
    // An explicit Copy Last Text/Copy Logs wins even if Windows reuses a handle.
    PENDING.with(|pending| *pending.borrow_mut() = None);
    unsafe {
        KillTimer(hwnd, RESTORE_TIMER);
    }
    Ok(())
}

/// Prepare a paste. Pending restoration holds off subsequent output, not the UI.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn prepare_paste(hwnd: HWND, text: &str) -> io::Result<()> {
    if PENDING.with(|pending| pending.borrow().is_some()) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "waiting for clipboard restoration",
        ));
    }
    let _open = Open::new(hwnd)?;
    let formats = unsafe { snapshot()? };
    // Install the restore timer before modifying the clipboard.
    if unsafe { SetTimer(hwnd, RESTORE_TIMER, 300, None) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { replace(text) };
    let paste_data = if result.is_ok() {
        unsafe { GetClipboardData(UNICODE_TEXT) }
    } else {
        std::ptr::null_mut()
    };
    // Publishing/closing the clipboard can itself update its sequence number.
    drop(_open);
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(Pending {
            formats,
            sequence: unsafe { GetClipboardSequenceNumber() },
            paste_data,
            emptied: false,
        })
    });
    result
}

/// Called by the controller's timer; a busy clipboard is retried on the next tick.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn restore(hwnd: HWND) -> io::Result<RestoreOutcome> {
    if PENDING.with(|cell| cell.borrow().is_none()) {
        unsafe {
            KillTimer(hwnd, RESTORE_TIMER);
        }
        return Ok(RestoreOutcome::Idle);
    }
    let _open = Open::new(hwnd)?;
    PENDING.with(|cell| {
        let mut pending = cell.borrow_mut();
        let mut outcome = RestoreOutcome::Idle;
        if let Some(saved) = pending.as_mut() {
            unsafe {
                if still_ours(
                    saved.emptied,
                    !saved.paste_data.is_null(),
                    GetClipboardSequenceNumber() == saved.sequence,
                    GetClipboardOwner() == hwnd,
                    !saved.emptied && GetClipboardData(UNICODE_TEXT) == saved.paste_data,
                ) {
                    outcome = RestoreOutcome::Restored;
                    if !saved.emptied {
                        if EmptyClipboard() == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        saved.emptied = true;
                    }
                    // Retry failed formats without emptying successful ones on the next tick.
                    for format in &mut saved.formats {
                        if !format.data.is_null() {
                            if SetClipboardData(format.id, format.data).is_null() {
                                saved.sequence = GetClipboardSequenceNumber();
                                return Err(io::Error::last_os_error());
                            }
                            format.data = std::ptr::null_mut();
                        }
                    }
                } else {
                    outcome = RestoreOutcome::Superseded;
                }
            }
        }
        *pending = None;
        unsafe {
            KillTimer(hwnd, RESTORE_TIMER);
        }
        Ok(outcome)
    })
}

#[cfg(test)]
mod tests {
    use super::still_ours;
    #[test]
    fn synthesized_formats_do_not_cancel_restoration() {
        assert!(still_ours(false, true, false, true, true));
        assert!(still_ours(false, true, true, true, true));
    }
    #[test]
    fn newer_copies_are_not_overwritten_even_with_matching_text() {
        assert!(!still_ours(false, true, false, false, true));
        assert!(!still_ours(false, true, false, true, false));
        assert!(!still_ours(false, true, true, false, false));
    }
    #[test]
    fn partial_restore_retries_require_unchanged_sequence() {
        assert!(still_ours(true, true, true, true, false));
        assert!(!still_ours(true, true, false, true, true));
        assert!(still_ours(false, false, true, true, false));
        assert!(!still_ours(false, false, false, true, false));
    }
}
