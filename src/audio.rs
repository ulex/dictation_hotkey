//! Event-driven shared-mode WASAPI. The Windows Audio Engine performs high-quality
//! channel/sample-rate conversion to 16 kHz mono PCM16; unsupported devices fail explicitly.
use crate::{
    logs_ui,
    spool::{MAX_AUDIO_BYTES, SAMPLE_RATE},
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
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
// WASAPI may mark the first packet discontinuous during normal stream startup.
fn is_discontinuity(first: bool, flags: u32) -> bool {
    !first && flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0
}

// Copy before ReleaseBuffer; a discontinuity does not invalidate the available PCM.
// Safety: non-null, non-silent data must point to at least frames * 2 readable bytes.
unsafe fn copy_packet(data: *const u8, frames: u32, flags: u32) -> io::Result<Vec<u8>> {
    if frames > SAMPLE_RATE {
        return Err(io::Error::other("oversized microphone packet"));
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
    let buffer_frames = client.GetBufferSize().map_err(error)?;
    let event = CreateEventW(None, false, false, None).map_err(error)?;
    let result = (|| {
        client.SetEventHandle(event).map_err(error)?;
        let reader: IAudioCaptureClient = client.GetService().map_err(error)?;
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        client.Start().map_err(error)?;
        let started = Instant::now();
        logs_ui::log(&format!(
            "WASAPI capture started (16 kHz mono, shared/event, buffer: {buffer_frames} frames)"
        ));
        let result = (|| {
            let mut pending = Vec::with_capacity(3200);
            let mut total = 0u64;
            let mut first = true;
            let mut last_end = None::<u64>;
            let mut packets = 0u64;
            let mut discontinuities = 0u64;
            let mut timeouts = 0u64;
            let mut last_packet_at = started;
            let mut max_packet_gap_ms = 0u128;
            loop {
                // Drain *before* Stop. Some capture drivers flag the next GetBuffer after
                // IAudioClient::Stop as a discontinuity even though the live stream was intact.
                if !stop.load(Ordering::Acquire) {
                    let wait = WaitForSingleObject(event, 100);
                    if wait == WAIT_TIMEOUT {
                        timeouts += 1;
                        continue;
                    }
                    if wait != WAIT_OBJECT_0 {
                        return Err(io::Error::other("microphone event wait failed"));
                    }
                }
                while reader.GetNextPacketSize().map_err(error)? != 0 {
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0;
                    let mut flags = 0;
                    let mut position = 0u64;
                    reader
                        .GetBuffer(
                            &mut data,
                            &mut frames,
                            &mut flags,
                            Some(&mut position),
                            None,
                        )
                        .map_err(error)?;
                    let now = Instant::now();
                    max_packet_gap_ms =
                        max_packet_gap_ms.max(now.duration_since(last_packet_at).as_millis());
                    last_packet_at = now;
                    packets += 1;
                    let copied = copy_packet(data, frames, flags);
                    // Release even on validation failure, and before potentially slow logging.
                    reader.ReleaseBuffer(frames).map_err(error)?;
                    let copied = copied?;
                    if is_discontinuity(first, flags) {
                        discontinuities += 1;
                        // Preserve available audio rather than failing the whole session.
                        // Do not synthesize silence from device positions: driver timestamps
                        // can jump/reset, especially with audio-engine format conversion.
                        if discontinuities <= 5 {
                            logs_ui::log(&format!(
                                "WASAPI warning: microphone discontinuity #{discontinuities}; continuing with available audio (some speech may be missing; elapsed: {} ms; stop requested: {}; packet: {frames} frames, flags: 0x{flags:x}; device position: {position}, previous end: {last_end:?}; captured: {} frames; packets: {packets}; buffer: {buffer_frames} frames; event timeouts: {timeouts}; max packet interval: {max_packet_gap_ms} ms){}",
                                started.elapsed().as_millis(), stop.load(Ordering::Acquire), total / 2,
                                if discontinuities == 5 { "; further discontinuity warnings suppressed; total reported at capture stop" } else { "" },
                            ));
                        }
                    }
                    first = false;
                    last_end = position.checked_add(u64::from(frames));
                    for sample in copied.as_chunks::<2>().0 {
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
                if stop.load(Ordering::Acquire) {
                    break;
                }
            }
            // No more GetBuffer calls after Stop: stopping can invalidate queued packets.
            client.Stop().map_err(error)?;
            if !pending.is_empty() {
                output(&pending)?;
            }
            logs_ui::log(&format!(
                "WASAPI capture stopped (elapsed: {} ms; captured: {} frames; packets: {packets}; discontinuities: {discontinuities}; event timeouts: {timeouts}; max packet interval: {max_packet_gap_ms} ms)",
                started.elapsed().as_millis(), total / 2,
            ));
            Ok(())
        })();
        if result.is_err() {
            let _ = client.Stop();
        }
        result
    })();
    let _ = CloseHandle(event);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discontinuous_packet_preserves_pcm_and_allows_following_packets() {
        let pcm = [0x12, 0x34, 0x56, 0x78];
        let discontinuity = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
        let mut captured = Vec::new();
        for flags in [0, discontinuity, discontinuity, 0] {
            captured.extend(unsafe { copy_packet(pcm.as_ptr(), 2, flags) }.unwrap());
        }
        assert_eq!(captured, pcm.repeat(4));
    }

    #[test]
    fn discontinuous_silent_packet_produces_silence_without_reading_data() {
        let flags =
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 | AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
        let copied = unsafe { copy_packet(std::ptr::null(), 160, flags) }.unwrap();
        assert_eq!(copied, vec![0; 320]);
    }

    #[test]
    fn discontinuity_does_not_hide_invalid_packets() {
        let flags = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
        assert!(unsafe { copy_packet(std::ptr::null(), 160, flags) }.is_err());
        assert!(unsafe {
            copy_packet(
                std::ptr::null(),
                SAMPLE_RATE + 1,
                flags | AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
            )
        }
        .is_err());
        assert!(unsafe { copy_packet(std::ptr::null(), 0, flags) }
            .unwrap()
            .is_empty());
    }

    #[test]
    fn startup_discontinuity_is_not_a_dropout() {
        assert!(!is_discontinuity(
            true,
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32
        ));
    }

    #[test]
    fn subsequent_discontinuity_is_detected_even_with_silence() {
        let discontinuity = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
        let silent = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
        assert!(is_discontinuity(false, discontinuity));
        assert!(is_discontinuity(false, discontinuity | silent));
        assert!(!is_discontinuity(false, silent));
        assert!(!is_discontinuity(false, 0));
    }
}
