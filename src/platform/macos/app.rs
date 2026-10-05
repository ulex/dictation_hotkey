//! Cocoa controller adapter. All state and output dispatch are owned by the main thread.
use crate::{
    bounded::Queue,
    config::Config,
    macos_bridge as ffi, paths,
    runtime::{self, Control, Event},
    session::{Controller, Phase, Ticket},
};
use std::{
    ffi::{c_char, c_void, CStr},
    fs::{self, File, OpenOptions},
    io,
    os::fd::AsRawFd,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};
struct App {
    config: Config,
    path: PathBuf,
    core: Controller,
    queue: Arc<Mutex<Queue<Event>>>,
    worker: Option<JoinHandle<()>>,
    control: Option<Control>,
    pending: String,
    terminal: Option<Ticket>,
    partial_fallback: bool,
    insertion_failed: bool,
    next_output: Instant,
}
fn error(message: &str) {
    if let Ok(text) = ffi::string(message) {
        unsafe { ffi::dh_ui_error(text.as_ptr()) }
    }
}
fn status(message: &str, busy: bool) {
    if let Ok(text) = ffi::string(message) {
        unsafe { ffi::dh_ui_status(text.as_ptr(), i32::from(busy)) }
    }
}
fn output(text: &str, mode: i32) -> bool {
    output_result(text, mode) == 0
}
fn output_result(text: &str, mode: i32) -> i32 {
    ffi::string(text).map_or(1, |text| unsafe { ffi::dh_output(text.as_ptr(), mode) })
}
impl App {
    fn save(&mut self, next: Config) -> io::Result<()> {
        next.validate()?;
        let json = ffi::string(&next.to_json()?)?;
        if unsafe { ffi::dh_ui_config(json.as_ptr()) } == 0 {
            return Err(io::Error::other("Settings were not saved."));
        }
        if let Err(e) = next.save(&self.path) {
            let old = ffi::string(&self.config.to_json()?)?;
            if unsafe { ffi::dh_ui_config(old.as_ptr()) } == 0 {
                return Err(io::Error::other("Settings save failed and shortcut/login rollback failed. Reopen Settings to reconcile the changes."));
            }
            return Err(e);
        }
        self.config = next;
        Ok(())
    }
    fn toggle(&mut self, clipboard: bool) -> io::Result<()> {
        if self.core.stop() {
            if let Some(control) = &self.control {
                control
                    .stop
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            status("Processing…", true);
            return Ok(());
        }
        if self.worker.is_some() {
            return Ok(());
        }
        self.core.reset();
        if self.config.string("api_key").is_empty() {
            return Err(io::Error::other("Enter your Mistral API key in Settings."));
        }
        self.config.validate()?;
        if !clipboard && unsafe { ffi::dh_can_insert() } == 0 {
            return Err(io::Error::other("Text insertion is not enabled for this build. Choose Enable Text Insertion in the menu, then allow this app in Accessibility. If it is already checked, remove the old entry and add the current app again, then quit and reopen it. Focus a text field before starting dictation. Record to Clipboard is available without this permission."));
        }
        let Some(ticket) = self
            .core
            .start(self.config.boolean("offline_mode"), clipboard)
        else {
            return Ok(());
        };
        self.pending.clear();
        self.terminal = None;
        self.partial_fallback = false;
        self.insertion_failed = false;
        let queue = self.queue.clone();
        let emit = Arc::new(move |event: Event| {
            let (bytes, terminal) = match &event {
                Event::Delta(_, text) => (text.len() + 64, false),
                Event::BatchDone(_, text) | Event::Failed(_, text) => (text.len() + 64, true),
                _ => (64, true),
            };
            queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event, bytes, terminal)
        });
        let control = Control::default();
        match runtime::spawn(self.config.clone(), ticket, control.clone(), emit) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.control = Some(control);
            }
            Err(e) => {
                self.core.fail(ticket, "session start failed");
                self.core.reset();
                return Err(e);
            }
        }
        status("Recording — shortcut or menu Stop to finish", true);
        Ok(())
    }
    fn flush(&mut self) {
        self.flush_with(output_result);
    }
    fn flush_with(&mut self, write: impl FnOnce(&str, i32) -> i32) {
        if self.pending.is_empty() || Instant::now() < self.next_output {
            return;
        }
        let end = self
            .pending
            .char_indices()
            .nth(160)
            .map_or(self.pending.len(), |(i, _)| i);
        let text = self.pending[..end].to_owned();
        // Conservative accounting: once output is attempted, fallback never reinserts a full result.
        self.core.output_succeeded(self.core.ticket);
        let result = write(
            &text,
            i32::from(self.config.string("typing_mode") == "keystrokes"),
        );
        if result != 0 {
            self.insertion_failed = true;
            self.core.clipboard_only = true;
            self.pending.clear();
            status(
                if result == 2 {
                    "Text insertion permission was lost — re-enable Accessibility for this build; transcript will be copied"
                } else {
                    "Insertion failed — full transcript will remain in Copy Last Text"
                },
                true,
            );
            return;
        }
        self.pending.drain(..end);
        self.next_output = Instant::now() + Duration::from_millis(100);
    }
    fn poll(&mut self) {
        // Process an entire bounded queue each tick. No UI closures accumulate per delta.
        loop {
            let event = self.queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
            let Some(event) = event else {
                break;
            };
            let ticket = event.ticket();
            match event {
                Event::Delta(_, text) => {
                    let before = self.core.text.len();
                    if self.core.delta(ticket, &text) {
                        self.pending.push_str(&self.core.text[before..]);
                    }
                    if self.core.phase == Phase::Failed {
                        if let Some(control) = &self.control {
                            control.cancel();
                        }
                        self.pending.clear();
                        status("Transcript limit reached", true);
                    }
                }
                Event::Fallback(_) => {
                    if ticket == self.core.ticket {
                        // Pending realtime output is discarded; already attempted output is tracked by core.
                        self.pending.clear();
                        self.partial_fallback = self.core.injected_prefix;
                        self.core.realtime_failure(ticket);
                        status("Recording — batch fallback", true);
                    }
                }
                Event::MicStopped(_) if ticket.session == self.core.ticket.session => {
                    self.core.stop();
                    status("Processing…", true);
                }
                Event::RealtimeDone(_) if ticket == self.core.ticket => {
                    self.core.stop();
                    self.terminal = Some(ticket);
                }
                Event::BatchDone(_, text) if ticket == self.core.ticket => {
                    self.core.stop();
                    if self.core.batch_result(ticket, text) {
                        self.pending.clone_from(&self.core.text);
                    }
                    self.terminal = Some(ticket);
                }
                Event::Failed(_, message) if ticket == self.core.ticket => {
                    self.pending.clear();
                    self.core.fail(ticket, "dictation failed");
                    self.terminal = Some(ticket);
                    status(&message, true);
                    // Keep errors in the bounded native log, without interrupting other apps.
                }
                _ => (),
            }
        }
        self.flush();
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) && self.pending.is_empty() {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            self.control = None;
            if let Some(ticket) = self.terminal.take() {
                if self.core.phase == Phase::Failed {
                    if !self.core.text.is_empty() {
                        self.core.last_text.clone_from(&self.core.text);
                    }
                    self.core.reset();
                    status("Failed — see Logs", false);
                } else {
                    let copied = self.core.clipboard_only
                        && !self.core.text.is_empty()
                        && output(&self.core.text, 2);
                    let partial = self.partial_fallback;
                    self.core.finish(ticket);
                    status(
                        if self.insertion_failed {
                            if copied {
                                "Insertion failed — transcript copied; see Logs"
                            } else {
                                "Insertion failed — transcript in Copy Last Text; see Logs"
                            }
                        } else if copied {
                            "Copied transcript"
                        } else if partial {
                            "Ready — fallback transcript in Copy Last Text"
                        } else {
                            "Ready"
                        },
                        false,
                    );
                }
            } else {
                self.core
                    .fail(self.core.ticket, "worker ended unexpectedly");
                self.core.reset();
                status("Failed — worker ended unexpectedly", false);
            }
        }
    }
    fn action(&mut self, code: i32, text: &str) -> io::Result<()> {
        match code {
            1 | 8 => self.toggle(code == 8)?,
            2 => {
                if self.core.stop() {
                    if let Some(control) = &self.control {
                        control
                            .stop
                            .store(true, std::sync::atomic::Ordering::Release);
                    }
                    status("Processing…", true);
                }
            }
            3 => {
                if !self.core.last_text.is_empty() && !output(&self.core.last_text, 2) {
                    return Err(io::Error::other("Clipboard copy failed"));
                }
            }
            4 if self.worker.is_none() => {
                let mut next = self.config.clone();
                next.set_bool("offline_mode", !next.boolean("offline_mode"));
                self.save(next)?;
            }
            5 if self.worker.is_none() => {
                let patch: serde_json::Value =
                    serde_json::from_str(text).map_err(io::Error::other)?;
                let mut next = self.config.clone();
                for (key, value) in patch
                    .as_object()
                    .ok_or_else(|| io::Error::other("invalid settings"))?
                {
                    if let Some(text) = value.as_str() {
                        next.set_string(key, text)?;
                    } else if let Some(flag) = value.as_bool() {
                        next.set_bool(key, flag);
                    }
                }
                self.save(next)?;
            }
            6 => self.poll(),
            7 => {
                if let Some(control) = &self.control {
                    control.cancel();
                }
            }
            _ => (),
        }
        Ok(())
    }
}
extern "C" fn action(context: *mut c_void, code: i32, text: *const c_char) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let app = unsafe { &mut *(context as *mut App) };
        let text = if text.is_null() {
            "".into()
        } else {
            unsafe { CStr::from_ptr(text) }.to_string_lossy()
        };
        app.action(code, &text)
    }));
    match result {
        Ok(Err(e)) => error(&e.to_string()),
        Err(_) => error("Application callback failed"),
        _ => (),
    }
}
fn instance(path: &std::path::Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    if unsafe { flock(lock.as_raw_fd(), 2 | 4) } != 0 {
        return Err(io::Error::other("Dictation Hotkey is already running"));
    }
    Ok(lock)
}
pub fn run() -> io::Result<()> {
    let path = paths::config()?;
    fs::create_dir_all(path.parent().unwrap())?;
    let _instance = instance(&path.with_extension("lock"))?;
    let config = Config::load(&path)?;
    let json = ffi::string(&config.to_json()?)?;
    let mut app = App {
        config,
        path,
        core: Controller::default(),
        queue: Arc::new(Mutex::new(Queue::default())),
        worker: None,
        control: None,
        pending: String::new(),
        terminal: None,
        partial_fallback: false,
        insertion_failed: false,
        next_output: Instant::now(),
    };
    unsafe { ffi::dh_run((&mut app as *mut App).cast(), action, json.as_ptr()) }
    if let Some(control) = app.control.take() {
        control.cancel();
    }
    if let Some(worker) = app.worker.take() {
        let _ = worker.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insertion_failure_retains_transcript_without_retrying_or_duplicating_fallback() {
        for failure in [1, 2] {
            let mut app = App {
                config: Config::default(),
                path: PathBuf::new(),
                core: Controller::default(),
                queue: Arc::new(Mutex::new(Queue::default())),
                worker: None,
                control: None,
                pending: String::new(),
                terminal: None,
                partial_fallback: false,
                insertion_failed: false,
                next_output: Instant::now(),
            };
            let ticket = app.core.start(false, false).unwrap();
            assert!(app.core.delta(ticket, "Dictated text 🎤"));
            app.pending.clone_from(&app.core.text);
            app.flush_with(|text, mode| {
                assert_eq!(text, "Dictated text 🎤");
                assert_eq!(mode, 0);
                failure
            });
            assert!(app.insertion_failed);
            assert!(app.core.clipboard_only);
            assert!(app.pending.is_empty());
            app.flush_with(|_, _| panic!("failed insertion must not be retried"));
            app.core.realtime_failure(ticket);
            assert!(app.core.stop());
            let fallback = app.core.ticket;
            assert!(!app
                .core
                .batch_result(fallback, "Complete dictated text 🎤".into()));
            app.core.finish(fallback);
            assert_eq!(app.core.last_text, "Complete dictated text 🎤");
        }
    }
    #[test]
    fn single_instance_lock_is_released_on_drop() {
        let path =
            std::env::temp_dir().join(format!("dh-macos-instance-{}.lock", std::process::id()));
        let first = instance(&path).unwrap();
        assert!(instance(&path).is_err());
        drop(first);
        drop(instance(&path).unwrap());
        fs::remove_file(path).unwrap();
    }
}
