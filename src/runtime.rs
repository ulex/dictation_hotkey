//! Session-scoped workers: capture -> bounded PCM queue -> disk spool -> bounded network queue.
//! Disk ownership is independent of network failure. Every operation has one terminal event.
use crate::{
    audio, config::Config, diagnostics, service, service_ws, session::Ticket, spool::Spool, wire,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, TrySendError},
        Arc, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Control {
    pub stop: Arc<AtomicBool>,
    pub abort: Arc<AtomicBool>,
    pub transport_abort: Arc<AtomicBool>,
    stop_requested_at: Arc<OnceLock<Instant>>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            transport_abort: Arc::new(AtomicBool::new(false)),
            stop_requested_at: Arc::new(OnceLock::new()),
        }
    }
}
impl Control {
    pub fn request_stop(&self) {
        self.stop_requested_at.get_or_init(Instant::now);
        self.stop.store(true, Ordering::Release);
    }
    fn mark_stop_if_missing(&self) {
        self.stop_requested_at.get_or_init(Instant::now);
    }
    pub fn stop_elapsed(&self) -> Option<Duration> {
        self.stop_requested_at.get().map(Instant::elapsed)
    }
    pub fn cancel(&self) {
        self.request_stop();
        self.abort.store(true, Ordering::Release);
        self.transport_abort.store(true, Ordering::Release);
    }
}
pub enum Event {
    Delta(Ticket, String),
    Fallback(Ticket),
    MicStopped(Ticket),
    RealtimeDone(Ticket),
    BatchDone(Ticket, String),
    Failed(Ticket, String),
}
impl Event {
    pub fn ticket(&self) -> Ticket {
        match self {
            Self::Delta(t, _)
            | Self::Fallback(t)
            | Self::MicStopped(t)
            | Self::RealtimeDone(t)
            | Self::BatchDone(t, _)
            | Self::Failed(t, _) => *t,
        }
    }
}
pub fn spawn(
    config: Config,
    ticket: Ticket,
    control: Control,
    emit: Arc<dyn Fn(Event) -> bool + Send + Sync>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("dictation-session".into())
        .spawn(move || {
            let fallback = Arc::new(AtomicBool::new(false));
            let operation = || Ticket {
                session: ticket.session,
                operation: ticket
                    .operation
                    .wrapping_add(u64::from(fallback.load(Ordering::Acquire))),
            };
            let run = || -> io::Result<()> {
                let offline = config.boolean("offline_mode");
                let session_started = Instant::now();
                let spool_started = Instant::now();
                let mut spool = Spool::create(&crate::paths::spool()?)?;
                if offline {
                    diagnostics::log(&format!(
                        "Offline timing: spool_create_ms={}",
                        spool_started.elapsed().as_millis()
                    ));
                }
                let realtime = !offline;
                let mut sender = None;
                let mut network = None;
                if realtime {
                    let (tx, rx) = sync_channel::<Vec<u8>>(50);
                    let cfg = config.clone();
                    let cancelled = control.transport_abort.clone();
                    let emit = emit.clone();
                    let failed = fallback.clone();
                    network = Some(
                        thread::Builder::new()
                            .name("dictation-realtime".into())
                            .spawn(move || {
                                let output = emit.clone();
                                let result = service_ws::realtime(
                                    cfg.string("api_key"),
                                    default(cfg.string("model"), wire::DEFAULT_MODEL),
                                    default(cfg.string("base_url"), wire::DEFAULT_URL),
                                    rx,
                                    cancelled,
                                    move |text| {
                                        if output(Event::Delta(ticket, text)) {
                                            Ok(())
                                        } else {
                                            Err(io::Error::other("output event queue full"))
                                        }
                                    },
                                );
                                if result.is_err() {
                                    failed.store(true, Ordering::Release);
                                    let _ = emit(Event::Fallback(ticket));
                                }
                                result
                            })?,
                    );
                    sender = Some(tx);
                }
                let (tx, rx) = sync_channel::<Vec<u8>>(20);
                let stop = control.stop.clone();
                let capture_started = Instant::now();
                let mic = match thread::Builder::new()
                    .name("dictation-capture".into())
                    .spawn(move || {
                        audio::capture(&stop, |pcm| {
                            tx.try_send(pcm.to_vec()).map_err(|_| {
                                io::Error::other("capture buffer overflow; recording incomplete")
                            })
                        })
                    }) {
                    Ok(mic) => mic,
                    Err(e) => {
                        control.transport_abort.store(true, Ordering::Release);
                        drop(sender);
                        if let Some(net) = network {
                            let _ = net.join();
                        }
                        return Err(e);
                    }
                };
                let mut disk_error = None;
                let mut spool_append_active = Duration::ZERO;
                let mut audio_bytes = 0u64;
                while let Ok(pcm) = rx.recv() {
                    if control.abort.load(Ordering::Acquire) {
                        control.request_stop();
                        break;
                    }
                    let append_started = offline.then(Instant::now);
                    let append_result = spool.append(&pcm);
                    if let Some(started) = append_started {
                        spool_append_active += started.elapsed();
                    }
                    if let Err(e) = append_result {
                        disk_error = Some(e);
                        control.request_stop();
                        break;
                    }
                    audio_bytes = audio_bytes.saturating_add(pcm.len() as u64);
                    if let Some(tx) = &sender {
                        match tx.try_send(pcm) {
                            Ok(()) => (),
                            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                                // Never silently keep streaming after dropping audio.
                                control.transport_abort.store(true, Ordering::Release);
                                sender.take();
                            }
                        }
                    }
                }
                drop(rx);
                let captured = mic
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("capture worker crashed")));
                control.mark_stop_if_missing();
                if offline {
                    diagnostics::log(&format!(
                        "Offline recording timing: capture_worker_ms={} spool_append_active_ms={} pcm_bytes={audio_bytes}",
                        capture_started.elapsed().as_millis(),
                        spool_append_active.as_millis(),
                    ));
                }
                let _ = emit(Event::MicStopped(operation()));
                drop(sender); // last captured packet is sent before flush/end
                if captured.is_err() || disk_error.is_some() {
                    control.transport_abort.store(true, Ordering::Release);
                }
                let transport = network.map(|net| {
                    net.join()
                        .unwrap_or_else(|_| Err(io::Error::other("realtime worker crashed")))
                });
                if realtime
                    && transport.as_ref().is_some_and(Result::is_err)
                    && !fallback.swap(true, Ordering::AcqRel)
                {
                    let _ = emit(Event::Fallback(ticket));
                }
                if let Some(e) = disk_error {
                    return Err(e);
                }
                captured?;
                if control.abort.load(Ordering::Acquire) {
                    return Err(io::Error::other("session cancelled"));
                }
                if realtime
                    && !fallback.load(Ordering::Acquire)
                    && transport.as_ref().is_some_and(Result::is_ok)
                {
                    let _ = emit(Event::RealtimeDone(ticket));
                    return Ok(());
                }
                let batch_ticket = operation();
                let text = if spool.is_empty() {
                    String::new()
                } else {
                    let finalize_started = Instant::now();
                    let finalized = spool.finish();
                    let finalize_elapsed = finalize_started.elapsed();
                    if offline {
                        diagnostics::log(&format!(
                            "Offline timing: wav_finalize_ms={}",
                            finalize_elapsed.as_millis()
                        ));
                    }
                    let wav = finalized?.to_owned();
                    let batch_started = Instant::now();
                    let result = service::batch(
                        &wav,
                        default(config.string("offline_model"), wire::DEFAULT_BATCH_MODEL),
                        config.string("api_key"),
                        control.abort.clone(),
                        offline,
                    );
                    if offline {
                        diagnostics::log(&format!(
                            "Offline transcription timing: result={} stop_to_result_ms={} batch_service_ms={} session_ms={}",
                            if result.is_ok() { "success" } else { "failed" },
                            control.stop_elapsed().map_or_else(
                                || "n/a".into(),
                                |elapsed| elapsed.as_millis().to_string()
                            ),
                            batch_started.elapsed().as_millis(),
                            session_started.elapsed().as_millis(),
                        ));
                    }
                    result?
                };
                let _ = emit(Event::BatchDone(batch_ticket, text));
                Ok(())
            };
            if let Err(e) = run() {
                control.request_stop();
                let _ = emit(Event::Failed(operation(), e.to_string()));
            }
        })
}
fn default<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_timestamp_is_set_once() {
        let control = Control::default();
        assert!(control.stop_elapsed().is_none());
        control.request_stop();
        let first = *control.stop_requested_at.get().unwrap();
        control.request_stop();
        assert!(control.stop.load(Ordering::Acquire));
        assert_eq!(control.stop_requested_at.get(), Some(&first));
    }
}
