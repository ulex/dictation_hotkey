//! Streamed batch upload over system WinHTTP/Schannel, off the UI thread.
use crate::{
    network_handle::Handle,
    spool::random_name,
    wire::{batch_text, Multipart},
};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use windows_sys::Win32::Networking::WinHttp::*;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn err(context: &str) -> io::Error {
    io::Error::other(format!("{context}: {}", io::Error::last_os_error()))
}
fn write_all(request: &Handle, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let mut count = 0;
        if unsafe {
            WinHttpWriteData(
                request.raw()?,
                bytes.as_ptr().cast(),
                bytes.len() as u32,
                &mut count,
            )
        } == 0
        {
            return Err(err("HTTP upload"));
        }
        if count == 0 || count as usize > bytes.len() {
            return Err(io::Error::other("invalid upload progress"));
        }
        bytes = &bytes[count as usize..];
    }
    Ok(())
}
/// Batch intentionally ignores the custom realtime host. No automatic billing-ambiguous retries.
pub fn batch(
    wav: &Path,
    model: &str,
    api_key: &str,
    cancelled: Arc<AtomicBool>,
) -> io::Result<String> {
    if api_key.is_empty() || api_key.chars().any(|c| c.is_control()) {
        return Err(io::Error::other("invalid API key"));
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::other("cancelled"));
    }
    let mut file = File::open(wav)?;
    let boundary = format!("dh-{}", random_name()?);
    let multipart = Multipart::new(&boundary, model, file.metadata()?.len())?;
    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                wide("DictationHotkey/1.0").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        if WinHttpSetTimeouts(session.raw()?, 15_000, 15_000, 15_000, 30_000) == 0 {
            return Err(err("WinHTTP timeouts"));
        }
        let connection = Handle::new(
            WinHttpConnect(session.raw()?, wide("api.mistral.ai").as_ptr(), 443, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.raw()?,
                wide("POST").as_ptr(),
                wide("/v1/audio/transcriptions").as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ),
            "HTTP request",
        )?;
        // Drop child before parent. Watchdog never holds the UI thread and covers read/write/connect.
        let _watch = request.watch(cancelled.clone(), Duration::from_secs(180))?;
        let disable = WINHTTP_DISABLE_REDIRECTS;
        if WinHttpSetOption(
            request.raw()?,
            WINHTTP_OPTION_DISABLE_FEATURE,
            &disable as *const _ as _,
            4,
        ) == 0
        {
            return Err(err("disable redirects"));
        }
        let auth = wide(&format!("Authorization: Bearer {api_key}"));
        if WinHttpAddRequestHeaders(
            request.raw()?,
            auth.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD,
        ) == 0
        {
            return Err(err("authentication header"));
        }
        let headers = wide(&format!(
            "Content-Type: multipart/form-data; boundary={boundary}\r\nAccept: application/json"
        ));
        if WinHttpSendRequest(
            request.raw()?,
            headers.as_ptr(),
            u32::MAX,
            null(),
            0,
            multipart.total,
            0,
        ) == 0
        {
            return Err(err("HTTP request"));
        }
        write_all(&request, multipart.prefix.as_bytes())?;
        let mut block = [0u8; 32768];
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::other("upload cancelled"));
            }
            let count = file.read(&mut block)?;
            if count == 0 {
                break;
            }
            write_all(&request, &block[..count])?;
        }
        write_all(&request, multipart.suffix.as_bytes())?;
        if WinHttpReceiveResponse(request.raw()?, null_mut()) == 0 {
            return Err(err("HTTP reply"));
        }
        let mut status = 0u32;
        let mut length = 4;
        if WinHttpQueryHeaders(
            request.raw()?,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut _ as _,
            &mut length,
            null_mut(),
        ) == 0
        {
            return Err(err("HTTP status"));
        }
        if status != 200 {
            return Err(io::Error::other(format!(
                "batch API HTTP {status} (check API key, quota and model)"
            )));
        }
        let mut body = Vec::new();
        loop {
            let mut count = 0;
            if WinHttpReadData(
                request.raw()?,
                block.as_mut_ptr().cast(),
                block.len() as u32,
                &mut count,
            ) == 0
            {
                return Err(err("HTTP response"));
            }
            if count == 0 {
                break;
            }
            if body.len() + count as usize > crate::session::MAX_TRANSCRIPT_BYTES + 65536 {
                return Err(io::Error::other("response exceeds limit"));
            }
            body.extend_from_slice(&block[..count as usize]);
        }
        batch_text(&body)
    }
}
