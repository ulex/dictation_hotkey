//! Shared realtime lifecycle: setup, silence warmup, audio, flush/end, and bounded shutdown.
use crate::{
    protocol::{self, Event},
    wire,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::Duration,
};
pub(crate) trait Transport: Send + Sync + Sized + 'static {
    type Watch;
    fn send(&self, message: &str) -> io::Result<()>;
    fn receive(&self) -> io::Result<Event>;
    fn close(&self);
    fn finish(&self) {
        self.close();
    }
    fn watch(
        self: &Arc<Self>,
        cancelled: Arc<AtomicBool>,
        timeout: Duration,
    ) -> io::Result<Self::Watch>;
}
pub(crate) fn stream<T: Transport>(
    ws: Arc<T>,
    audio: Receiver<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    mut delta: impl FnMut(String) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let handshake_watch = ws.watch(cancelled.clone(), Duration::from_secs(15))?;
    loop {
        match ws.receive()? {
            Event::Created => break,
            Event::Error(_) => return Err(io::Error::other("realtime service rejected session")),
            Event::Delta(text) => delta(text)?,
            Event::Done => return Err(io::Error::other("realtime ended during handshake")),
            Event::Unknown => (),
        }
    }
    ws.send(&protocol::session_update().to_string())?;
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
                match recv_ws.receive()? {
                    Event::Delta(text) => delta(text)?,
                    Event::Done if recv_end.load(Ordering::Acquire) => return Ok(()),
                    Event::Done => {
                        return Err(io::Error::other("realtime ended before microphone stopped"))
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
            ws.send(&wire::append(&[0; 3200])?)?;
            std::thread::sleep(Duration::from_millis(50));
        }
        loop {
            if cancelled.load(Ordering::Acquire) || finished.load(Ordering::Acquire) {
                return Err(io::Error::other("realtime interrupted"));
            }
            match audio.recv_timeout(Duration::from_millis(50)) {
                Ok(pcm) => ws.send(&wire::append(&pcm)?)?,
                Err(RecvTimeoutError::Timeout) => (),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        ws.send(&protocol::flush().to_string())?;
        sent_end.store(true, Ordering::Release);
        ws.send(&protocol::end().to_string())
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
    ws.finish();
    sending?;
    received?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Mutex};
    struct Fake {
        events: Mutex<mpsc::Receiver<Event>>,
        replies: mpsc::Sender<Event>,
        sent: Mutex<Vec<String>>,
        early_done: bool,
    }
    impl Transport for Fake {
        type Watch = ();
        fn send(&self, text: &str) -> io::Result<()> {
            self.sent.lock().unwrap().push(text.into());
            if serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .as_ref()
                == Some(&protocol::end())
            {
                self.replies.send(Event::Done).unwrap();
            }
            if self.early_done && text.contains("session.update") {
                self.replies.send(Event::Done).unwrap();
            }
            Ok(())
        }
        fn receive(&self) -> io::Result<Event> {
            self.events
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .map_err(io::Error::other)
        }
        fn close(&self) {
            let _ = self.replies.send(Event::Error("closed".into()));
        }
        fn watch(self: &Arc<Self>, _: Arc<AtomicBool>, _: Duration) -> io::Result<()> {
            Ok(())
        }
    }
    fn fake(early_done: bool) -> Arc<Fake> {
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Created).unwrap();
        Arc::new(Fake {
            events: Mutex::new(rx),
            replies: tx,
            sent: Mutex::new(Vec::new()),
            early_done,
        })
    }
    #[test]
    fn warmup_precedes_real_audio_and_flush_end() {
        let socket = fake(false);
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(vec![1, 2]).unwrap();
        drop(tx);
        stream(socket.clone(), rx, Arc::new(AtomicBool::new(false)), |_| {
            Ok(())
        })
        .unwrap();
        let sent = socket.sent.lock().unwrap();
        assert_eq!(sent[0], protocol::session_update().to_string());
        assert_eq!(sent.len(), 24);
        for message in &sent[1..21] {
            assert_eq!(message, &wire::append(&[0; 3200]).unwrap());
        }
        assert_eq!(sent[21], wire::append(&[1, 2]).unwrap());
        assert_eq!(sent[22], protocol::flush().to_string());
        assert_eq!(sent[23], protocol::end().to_string());
    }
    #[test]
    fn early_provider_done_is_failure() {
        let socket = fake(true);
        let (_tx, rx) = mpsc::sync_channel(1);
        assert!(stream(socket, rx, Arc::new(AtomicBool::new(false)), |_| Ok(())).is_err());
    }
    #[test]
    fn cancellation_stops_before_microphone_audio() {
        let socket = fake(false);
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(vec![1, 2]).unwrap();
        drop(tx);
        assert!(
            stream(socket.clone(), rx, Arc::new(AtomicBool::new(true)), |_| Ok(
                ()
            ))
            .is_err()
        );
        assert_eq!(socket.sent.lock().unwrap().len(), 1);
    }
}
