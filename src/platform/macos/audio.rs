use crate::macos_bridge;
use std::{
    ffi::c_void,
    io,
    sync::atomic::{AtomicBool, Ordering},
};
struct Capture<'a, F> {
    stop: &'a AtomicBool,
    emit: F,
    error: Option<io::Error>,
}
extern "C" fn stopped<F>(context: *mut c_void) -> i32 {
    let capture = unsafe { &*(context as *const Capture<'_, F>) };
    i32::from(capture.stop.load(Ordering::Acquire))
}
extern "C" fn pcm<F: FnMut(&[u8]) -> io::Result<()> + Send>(
    context: *mut c_void,
    data: *const u8,
    len: usize,
) -> i32 {
    // Swift serializes tap callbacks and waits for them before returning dh_capture.
    let capture = unsafe { &mut *(context as *mut Capture<'_, F>) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (capture.emit)(unsafe { std::slice::from_raw_parts(data, len) })
    }))
    .unwrap_or_else(|_| Err(io::Error::other("capture callback panicked")));
    if let Err(error) = result {
        capture.error = Some(error);
        return 0;
    }
    1
}
pub fn capture(
    stop: &AtomicBool,
    emit: impl FnMut(&[u8]) -> io::Result<()> + Send,
) -> io::Result<()> {
    fn run<F: FnMut(&[u8]) -> io::Result<()> + Send>(stop: &AtomicBool, emit: F) -> io::Result<()> {
        let mut capture = Capture {
            stop,
            emit,
            error: None,
        };
        let result = unsafe {
            macos_bridge::dh_capture(
                (&mut capture as *mut Capture<'_, F>).cast(),
                stopped::<F>,
                pcm::<F>,
            )
        };
        if let Some(error) = capture.error {
            return Err(error);
        }
        match result {
            0 => Ok(()),
            2 => Err(io::Error::other("Microphone permission denied. Enable Dictation Hotkey in System Settings > Privacy & Security > Microphone.")),
            _ => Err(io::Error::other("Microphone capture failed (check the default input device).")),
        }
    }
    run(stop, emit)
}
