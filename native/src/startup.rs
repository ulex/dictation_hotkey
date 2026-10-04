//! Explicit per-user Startup shortcut changes; rollback restores the original shortcut bytes,
//! not a reconstructed link that might accidentally replace the Python application's target.
use std::{io, os::windows::ffi::OsStrExt, path::PathBuf};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER},
        UI::Shell::{FOLDERID_Startup, IShellLinkW, SHGetKnownFolderPath, ShellLink},
    },
};
fn err(e: windows::core::Error) -> io::Error {
    io::Error::other(format!("Startup shortcut: {e}"))
}
fn wide(path: &std::ffi::OsStr) -> Vec<u16> {
    path.encode_wide().chain(Some(0)).collect()
}
fn shortcut() -> io::Result<PathBuf> {
    unsafe {
        let folder =
            SHGetKnownFolderPath(&FOLDERID_Startup, Default::default(), None).map_err(err)?;
        let name = folder.to_string();
        CoTaskMemFree(Some(folder.0.cast()));
        Ok(PathBuf::from(name.map_err(io::Error::other)?).join("Dictation Hotkey.lnk"))
    }
}
pub fn snapshot() -> io::Result<Option<Vec<u8>>> {
    let path = shortcut()?;
    match std::fs::File::open(path) {
        Ok(file) => {
            use io::Read;
            let mut data = Vec::new();
            file.take(65537).read_to_end(&mut data)?;
            if data.len() > 65536 {
                return Err(io::Error::other("Startup shortcut exceeds backup limit"));
            }
            Ok(Some(data))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn restore(snapshot: Option<Vec<u8>>) -> io::Result<()> {
    if let Some(data) = snapshot {
        std::fs::write(shortcut()?, data)
    } else {
        set(false)
    }
}
pub fn set(enabled: bool) -> io::Result<()> {
    let path = shortcut()?;
    if !enabled {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }
    unsafe {
        let exe = std::env::current_exe()?;
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        link.SetPath(PCWSTR(wide(exe.as_os_str()).as_ptr()))
            .map_err(err)?;
        if let Some(parent) = exe.parent() {
            link.SetWorkingDirectory(PCWSTR(wide(parent.as_os_str()).as_ptr()))
                .map_err(err)?;
        }
        let persist: IPersistFile = link.cast().map_err(err)?;
        persist
            .Save(PCWSTR(wide(path.as_os_str()).as_ptr()), true)
            .map_err(err)
    }
}
