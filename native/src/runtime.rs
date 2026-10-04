//! Session-scoped workers: capture -> bounded PCM queue -> disk spool -> bounded network queue.
//! Disk ownership is independent of network failure. Every operation has one terminal event.
use crate::{audio, config::Config, service, service_ws, session::Ticket, spool::Spool, wire};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
};
#[derive(Clone)]
pub struct Control {
    pub stop: Arc<AtomicBool>,
    pub abort: Arc<AtomicBool>,
    pub transport_abort: Arc<AtomicBool>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            transport_abort: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Control {
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
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
                let mut spool = Spool::create(&crate::paths::spool()?)?;
                let realtime = !config.boolean("offline_mode");
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
                while let Ok(pcm) = rx.recv() {
                    if control.abort.load(Ordering::Acquire) {
                        control.stop.store(true, Ordering::Release);
                        break;
                    }
                    if let Err(e) = spool.append(&pcm) {
                        disk_error = Some(e);
                        control.stop.store(true, Ordering::Release);
                        break;
                    }
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
                    let wav = spool.finish()?.to_owned();
                    service::batch(
                        &wav,
                        default(config.string("offline_model"), wire::DEFAULT_BATCH_MODEL),
                        config.string("api_key"),
                        control.abort.clone(),
                    )?
                };
                let _ = emit(Event::BatchDone(batch_ticket, text));
                Ok(())
            };
            if let Err(e) = run() {
                control.stop.store(true, Ordering::Release);
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
