//! Identify the foreground RDP client without relying on localized window titles.
fn is_rdp_client(image_path: &str) -> bool {
    let name = image_path.rsplit(['\\', '/']).next().unwrap_or_default();
    // Classic Remote Desktop Connection and the newer Microsoft RDP client.
    ["mstsc.exe", "msrdc.exe"]
        .iter()
        .any(|client| name.eq_ignore_ascii_case(client))
}

#[cfg(windows)]
pub fn foreground_is_rdp() -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if pid == 0 {
            return false;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        // Bounded path query; an inaccessible process or oversized path is not
        // treated as RDP. Always release the process handle before returning.
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let queried = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len);
        CloseHandle(process);
        queried != 0 && is_rdp_client(&String::from_utf16_lossy(&path[..len as usize]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_clients_case_insensitively() {
        for path in [
            r"C:\Windows\System32\mstsc.exe",
            r"C:\Program Files\Remote Desktop\MSRDC.EXE",
            "mstsc.exe",
            "C:/Windows/System32/MsTsC.ExE",
        ] {
            assert!(is_rdp_client(path), "{path}");
        }
    }

    #[test]
    fn does_not_match_titles_directories_or_similar_names() {
        for path in [
            "",
            "Remote Desktop Connection",
            r"C:\mstsc.exe\notepad.exe",
            "not-mstsc.exe",
            "mstsc.exe.bak",
            "msrdcw.exe",
        ] {
            assert!(!is_rdp_client(path), "{path}");
        }
    }
}
