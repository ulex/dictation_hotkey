//! Native URLSession WebSocket transport using the shared provider framing.
use crate::{
    macos_bridge as ffi,
    protocol::{self, Event},
    service::Handle,
    wire,
};
use std::{
    io,
    sync::{atomic::AtomicBool, mpsc::Receiver, Arc},
    time::Duration,
};
fn send(ws: &Handle, message: &str) -> io::Result<()> {
    let message = ffi::string(message)?;
    if unsafe { ffi::dh_ws_send(ws.raw(), message.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::other("WebSocket send failed"))
    }
}
fn recv(ws: &Handle) -> io::Result<Event> {
    let mut bytes = vec![0; protocol::MAX_EVENT_BYTES];
    let count = unsafe { ffi::dh_ws_receive(ws.raw(), bytes.as_mut_ptr(), bytes.len()) };
    if count < 0 {
        return Err(io::Error::other("WebSocket receive failed"));
    }
    protocol::parse_event(&bytes[..count as usize])
}
pub fn realtime(
    api_key: &str,
    model: &str,
    base: &str,
    audio: Receiver<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    delta: impl FnMut(String) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    if api_key.is_empty() || api_key.chars().any(char::is_control) {
        return Err(io::Error::other("invalid API key"));
    }
    let (host, port, path) = wire::endpoint(base, model)?;
    let url = ffi::string(&format!("wss://{host}:{port}{path}"))?;
    let key = ffi::string(api_key)?;
    let ws = Handle::new(unsafe { ffi::dh_ws_new(url.as_ptr(), key.as_ptr()) })?;
    crate::realtime::stream(ws, audio, cancelled, delta)
}
impl crate::realtime::Transport for Handle {
    type Watch = crate::service::Watch;
    fn send(&self, message: &str) -> io::Result<()> {
        send(self, message)
    }
    fn receive(&self) -> io::Result<Event> {
        recv(self)
    }
    fn close(&self) {
        Handle::close(self);
    }
    fn watch(
        self: &Arc<Self>,
        cancelled: Arc<AtomicBool>,
        timeout: Duration,
    ) -> io::Result<Self::Watch> {
        Handle::watch(self, cancelled, timeout)
    }
}
