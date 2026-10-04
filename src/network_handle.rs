//! Shared WinHTTP ownership. Closing a handle cancels pending synchronous operations.
//! A handle is closed exactly once; workers are joined before parent handles are released.
use std::{
    ffi::c_void,
    io,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

pub struct Handle(Arc<AtomicUsize>);
impl Handle {
    pub fn new(raw: *mut c_void, context: &str) -> io::Result<Self> {
        if raw.is_null() {
            return Err(io::Error::other(format!(
                "{context}: {}",
                io::Error::last_os_error()
            )));
        }
        Ok(Self(Arc::new(AtomicUsize::new(raw as usize))))
    }
    pub fn raw(&self) -> io::Result<*mut c_void> {
        let raw = self.0.load(Ordering::Acquire);
        if raw == 0 {
            return Err(io::Error::other("network operation cancelled or timed out"));
        }
        Ok(raw as *mut c_void)
    }
    pub fn close(&self) {
        close(&self.0);
    }
    pub fn watch(&self, cancelled: Arc<AtomicBool>, timeout: Duration) -> io::Result<Watchdog> {
        let raw = self.0.clone();
        let (tx, rx) = mpsc::channel();
        let deadline = Instant::now() + timeout;
        let worker = std::thread::Builder::new()
            .name("dictation-network-deadline".into())
            .spawn(move || loop {
                if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                    close(&raw);
                    break;
                }
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => (),
                }
            })?;
        Ok(Watchdog {
            tx,
            worker: Some(worker),
        })
    }
}
fn close(raw: &AtomicUsize) {
    let handle = raw.swap(0, Ordering::AcqRel) as *mut c_void;
    if !handle.is_null() {
        unsafe {
            WinHttpCloseHandle(handle);
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.close();
    }
}
pub struct Watchdog {
    tx: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.tx.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
