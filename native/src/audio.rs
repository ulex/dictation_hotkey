//! Event-driven shared-mode WASAPI. The Windows Audio Engine performs high-quality
//! channel/sample-rate conversion to 16 kHz mono PCM16; unsupported devices fail explicitly.
use crate::spool::{MAX_AUDIO_BYTES, SAMPLE_RATE};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use windows::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::Audio::{
        eCapture, eConsole, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
        WAVE_FORMAT_PCM,
    },
    System::{
        Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED},
        Threading::{CreateEventW, WaitForSingleObject},
    },
};
fn error(e: windows::core::Error) -> io::Error {
    io::Error::other(format!(
        "microphone: {e} (check device and Windows microphone permissions)"
    ))
}
pub fn capture(
    stop: &AtomicBool,
    mut on_chunk: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(error)?;
        let result = capture_inner(stop, &mut on_chunk);
        CoUninitialize();
        result
    }
}
unsafe fn capture_inner(
    stop: &AtomicBool,
    output: &mut impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(error)?;
    let device = enumerator
        .GetDefaultAudioEndpoint(eCapture, eConsole)
        .map_err(error)?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(error)?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    client
        .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, &format, None)
        .map_err(error)?;
    let event = CreateEventW(None, false, false, None).map_err(error)?;
    let result = (|| {
        client.SetEventHandle(event).map_err(error)?;
        let reader: IAudioCaptureClient = client.GetService().map_err(error)?;
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        client.Start().map_err(error)?;
        let result = (|| {
            let mut pending = Vec::with_capacity(3200);
            let mut total = 0u64;
            let mut first = true;
            let mut stopped = false;
            loop {
                if !stopped {
                    if stop.load(Ordering::Acquire) {
                        client.Stop().map_err(error)?;
                        stopped = true; // drain the final buffered microphone packets below
                    } else {
                        let wait = WaitForSingleObject(event, 100);
                        if wait == WAIT_TIMEOUT {
                            continue;
                        }
                        if wait != WAIT_OBJECT_0 {
                            return Err(io::Error::other("microphone event wait failed"));
                        }
                    }
                }
                while reader.GetNextPacketSize().map_err(error)? != 0 {
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0;
                    let mut flags = 0;
                    reader
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .map_err(error)?;
                    let copied = (|| {
                        if frames > SAMPLE_RATE {
                            return Err(io::Error::other("oversized microphone packet"));
                        }
                        if !first && flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                            return Err(io::Error::other(
                                "microphone discontinuity; recording incomplete",
                            ));
                        }
                        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            Ok(vec![0u8; frames as usize * 2])
                        } else if frames == 0 {
                            Ok(Vec::new())
                        } else if data.is_null() {
                            Err(io::Error::other("invalid microphone buffer"))
                        } else {
                            Ok(std::slice::from_raw_parts(data, frames as usize * 2).to_vec())
                        }
                    })();
                    reader.ReleaseBuffer(frames).map_err(error)?;
                    first = false;
                    for sample in copied?.as_chunks::<2>().0 {
                        if total == MAX_AUDIO_BYTES {
                            stop.store(true, Ordering::Release);
                            break;
                        }
                        total += 2;
                        pending.extend_from_slice(sample);
                        if pending.len() == 3200 {
                            output(&pending)?;
                            pending.clear();
                        }
                    }
                }
                if total == MAX_AUDIO_BYTES {
                    stop.store(true, Ordering::Release);
                }
                if stopped {
                    break;
                }
            }
            if !pending.is_empty() {
                output(&pending)?;
            }
            Ok(())
        })();
        let _ = client.Stop();
        result
    })();
    let _ = CloseHandle(event);
    result
}
