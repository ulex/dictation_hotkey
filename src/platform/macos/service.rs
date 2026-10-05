//! Disk-streamed multipart upload through URLSession. No recording-sized RAM buffer.
use crate::{
    macos_bridge as ffi,
    spool::random_name,
    wire::{self, Multipart},
};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Handle(*mut c_void);
// Native network handles synchronize their state; URLSession permits concurrent send/receive/cancel.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub(crate) fn new(raw: *mut c_void) -> io::Result<Arc<Self>> {
        if raw.is_null() {
            Err(io::Error::other("network setup failed"))
        } else {
            Ok(Arc::new(Self(raw)))
        }
    }
    pub(crate) fn raw(&self) -> *mut c_void {
        self.0
    }
    pub(crate) fn close(&self) {
        unsafe { ffi::dh_net_close(self.0) }
    }
    pub(crate) fn watch(
        self: &Arc<Self>,
        cancelled: Arc<AtomicBool>,
        timeout: Duration,
    ) -> io::Result<Watch> {
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let handle = self.clone();
        let worker = thread::Builder::new()
            .name("dictation-network-watch".into())
            .spawn(move || {
                let deadline = Instant::now() + timeout;
                while !finished.load(Ordering::Acquire) {
                    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                        handle.close();
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            })?;
        Ok(Watch {
            done,
            worker: Some(worker),
        })
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { ffi::dh_net_free(self.0) }
    }
}
pub(crate) struct Watch {
    done: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
struct Upload(PathBuf);
impl Drop for Upload {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn batch(
    wav: &Path,
    model: &str,
    api_key: &str,
    cancelled: Arc<AtomicBool>,
) -> io::Result<String> {
    if api_key.is_empty() || api_key.chars().any(char::is_control) {
        return Err(io::Error::other("invalid API key"));
    }
    let boundary = format!("dh-{}", random_name()?);
    let mut source = File::open(wav)?;
    let multipart = Multipart::new(&boundary, model, source.metadata()?.len())?;
    let path = wav.with_file_name(format!("dh-{}.upload", random_name()?));
    use std::os::unix::fs::OpenOptionsExt;
    let mut destination = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let upload = Upload(path);
    destination.write_all(multipart.prefix.as_bytes())?;
    // Copy in bounded chunks and remain cancellable while staging the multipart file.
    use io::Read;
    let mut block = [0u8; 32768];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::other("upload cancelled"));
        }
        let count = source.read(&mut block)?;
        if count == 0 {
            break;
        }
        destination.write_all(&block[..count])?;
    }
    destination.write_all(multipart.suffix.as_bytes())?;
    drop(destination);
    let path = ffi::string(
        upload
            .0
            .to_str()
            .ok_or_else(|| io::Error::other("invalid upload path"))?,
    )?;
    let boundary = ffi::string(&boundary)?;
    let key = ffi::string(api_key)?;
    let handle =
        Handle::new(unsafe { ffi::dh_http_new(path.as_ptr(), boundary.as_ptr(), key.as_ptr()) })?;
    let _watch = handle.watch(cancelled, Duration::from_secs(180))?;
    let mut response = vec![0; crate::session::MAX_TRANSCRIPT_BYTES + 65536];
    let count =
        unsafe { ffi::dh_http_receive(handle.raw(), response.as_mut_ptr(), response.len()) };
    if count < 0 {
        return Err(io::Error::other(
            "batch request failed (check API key, quota, model and connection)",
        ));
    }
    wire::batch_text(&response[..count as usize])
}
