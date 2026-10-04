//! SDK-equivalent realtime framing over WinHTTP. One sender and one receiver;
//! handle-close watchdogs cancel pending operations without blocking the UI thread.
use crate::{
    network_handle::Handle,
    protocol::{self, Assembler, Event},
    wire,
};
use std::{
    io,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::Duration,
};
use windows_sys::Win32::Networking::WinHttp::*;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn fail(name: &str) -> io::Error {
    io::Error::other(format!("{name}: {}", io::Error::last_os_error()))
}
fn send(ws: &Handle, message: &str) -> io::Result<()> {
    let status = unsafe {
        WinHttpWebSocketSend(
            ws.raw()?,
            WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE,
            message.as_ptr().cast(),
            message.len() as u32,
        )
    };
    if status != 0 {
        return Err(io::Error::other(format!("WebSocket send: {status}")));
    }
    Ok(())
}
fn recv(ws: &Handle) -> io::Result<Event> {
    let mut message = Assembler::default();
    let mut fragment_watch = None;
    loop {
        let mut block = [0u8; 8192];
        let mut count = 0;
        let mut kind = WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE;
        let status = unsafe {
            WinHttpWebSocketReceive(
                ws.raw()?,
                block.as_mut_ptr().cast(),
                block.len() as u32,
                &mut count,
                &mut kind,
            )
        };
        // Silence may produce no deltas. Do not mistake a per-read timeout for lost audio;
        // the independent handshake/finalization/cancellation watchdog owns deadlines.
        if status == 12002 && message.is_empty() {
            continue;
        }
        if status != 0 {
            return Err(io::Error::other(format!("WebSocket receive: {status}")));
        }
        if kind == WINHTTP_WEB_SOCKET_CLOSE_BUFFER_TYPE {
            return Err(io::Error::other(
                "realtime closed before transcription.done",
            ));
        }
        if kind != WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE
            && kind != WINHTTP_WEB_SOCKET_UTF8_FRAGMENT_BUFFER_TYPE
        {
            return Err(io::Error::other("unexpected WebSocket frame"));
        }
        if count as usize > block.len() {
            return Err(io::Error::other("invalid realtime frame size"));
        }
        if let Some(event) = message.push(
            &block[..count as usize],
            kind == WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE,
        )? {
            return Ok(event);
        }
        if fragment_watch.is_none() {
            fragment_watch =
                Some(ws.watch(Arc::new(AtomicBool::new(false)), Duration::from_secs(30))?);
        }
    }
}
pub fn realtime(
    api_key: &str,
    model: &str,
    base: &str,
    audio: Receiver<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    mut delta: impl FnMut(String) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let (host, port, path) = wire::endpoint(base, model)?;
    if api_key.is_empty() || api_key.chars().any(|c| c.is_control()) {
        return Err(io::Error::other("invalid API key"));
    }
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
        if WinHttpSetTimeouts(session.raw()?, 15_000, 15_000, 15_000, 15_000) == 0 {
            return Err(fail("WebSocket timeouts"));
        }
        let connection = Handle::new(
            WinHttpConnect(session.raw()?, wide(&host).as_ptr(), port, 0),
            "WinHttpConnect",
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connection.raw()?,
                wide("GET").as_ptr(),
                wide(&path).as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WebSocket request",
        )?;
        let request_watch = request.watch(cancelled.clone(), Duration::from_secs(30))?;
        let disable = WINHTTP_DISABLE_REDIRECTS;
        if WinHttpSetOption(
            request.raw()?,
            WINHTTP_OPTION_DISABLE_FEATURE,
            &disable as *const _ as _,
            4,
        ) == 0
        {
            return Err(fail("disable redirects"));
        }
        let auth = wide(&format!("Authorization: Bearer {api_key}"));
        if WinHttpAddRequestHeaders(
            request.raw()?,
            auth.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD,
        ) == 0
        {
            return Err(fail("authentication"));
        }
        if WinHttpSetOption(
            request.raw()?,
            WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET,
            null(),
            0,
        ) == 0
        {
            return Err(fail("WebSocket upgrade"));
        }
        if WinHttpSendRequest(request.raw()?, null(), 0, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.raw()?, null_mut()) == 0
        {
            return Err(fail("WebSocket handshake"));
        }
        let mut status = 0u32;
        let mut len = 4;
        if WinHttpQueryHeaders(
            request.raw()?,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut _ as _,
            &mut len,
            null_mut(),
        ) == 0
        {
            return Err(fail("WebSocket HTTP status"));
        }
        if status != 101 {
            return Err(io::Error::other(format!(
                "realtime API HTTP {status} (check API key, quota and model)"
            )));
        }
        let ws = Arc::new(Handle::new(
            WinHttpWebSocketCompleteUpgrade(request.raw()?, 0),
            "WebSocket upgrade",
        )?);
        drop(request_watch);
        let handshake_watch = ws.watch(cancelled.clone(), Duration::from_secs(15))?;
        loop {
            match recv(&ws)? {
                Event::Created => break,
                Event::Error(_) => {
                    return Err(io::Error::other("realtime service rejected session"))
                }
                Event::Delta(text) => delta(text)?,
                Event::Done => return Err(io::Error::other("realtime ended during handshake")),
                Event::Unknown => (),
            }
        }
        send(&ws, &protocol::session_update().to_string())?;
        drop(handshake_watch);
        let _lifetime_watch = ws.watch(
            cancelled.clone(),
            Duration::from_secs(crate::spool::MAX_SECONDS + 60),
        )?;
        let finished = Arc::new(AtomicBool::new(false));
        let sent_end = Arc::new(AtomicBool::new(false));
        let recv_ws = ws.clone();
        let recv_finished = finished.clone();
        let recv_end = sent_end.clone();
        let reader = std::thread::Builder::new()
            .name("dictation-receiver".into())
            .spawn(move || {
                let result = (|| loop {
                    match recv(&recv_ws)? {
                        Event::Delta(text) => delta(text)?,
                        Event::Done if recv_end.load(Ordering::Acquire) => return Ok(()),
                        Event::Done => {
                            return Err(io::Error::other(
                                "realtime ended before microphone stopped",
                            ))
                        }
                        Event::Error(_) => return Err(io::Error::other("realtime service error")),
                        _ => (),
                    }
                })();
                recv_finished.store(true, Ordering::Release);
                if result.is_err() {
                    recv_ws.close();
                }
                result
            })?;
        let sending = (|| {
            for _ in 0..20 {
                if cancelled.load(Ordering::Acquire) || finished.load(Ordering::Acquire) {
                    return Err(io::Error::other("realtime interrupted"));
                }
                send(&ws, &wire::append(&[0; 3200])?)?;
                std::thread::sleep(Duration::from_millis(50));
            }
            loop {
                if cancelled.load(Ordering::Acquire) || finished.load(Ordering::Acquire) {
                    return Err(io::Error::other("realtime interrupted"));
                }
                match audio.recv_timeout(Duration::from_millis(50)) {
                    Ok(pcm) => send(&ws, &wire::append(&pcm)?)?,
                    Err(RecvTimeoutError::Timeout) => (),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            send(&ws, &protocol::flush().to_string())?;
            sent_end.store(true, Ordering::Release);
            send(&ws, &protocol::end().to_string())
        })();
        if sending.is_err() {
            ws.close();
        }
        let final_watch = match ws.watch(cancelled, Duration::from_secs(30)) {
            Ok(watch) => watch,
            Err(e) => {
                ws.close();
                let _ = reader.join();
                return Err(e);
            }
        };
        let received = reader
            .join()
            .map_err(|_| io::Error::other("realtime receiver crashed"));
        drop(final_watch);
        // Shutdown is a send operation, so only do it after sender and receiver have ended.
        if let Ok(raw) = ws.raw() {
            let _ = WinHttpWebSocketShutdown(raw, 1000, null(), 0);
        }
        ws.close();
        sending?;
        received?
    }
}
