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
// Device positions are in capture frames. A discontinuity flag can occur with or
// without a measurable gap; only forward jumps can be repaired with silence.
fn missing_frames(previous_end: Option<u64>, position: u64) -> u64 {
    previous_end.map_or(0, |end| position.saturating_sub(end))
}

fn append_pcm(
    pcm: &[u8],
    total: &mut u64,
    pending: &mut Vec<u8>,
    stop: &AtomicBool,
    output: &mut impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    for sample in pcm.as_chunks::<2>().0 {
        if *total == MAX_AUDIO_BYTES {
            stop.store(true, Ordering::Release);
            break;
        }
        *total += 2;
        pending.extend_from_slice(sample);
        if pending.len() == 3200 {
            output(pending)?;
            pending.clear();
        }
    }
    Ok(())
}

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
            let mut last_end = None::<u64>;
            let mut discontinuities = 0u64;
            let mut concealed_frames = 0u64;
            let mut packets = 0u64;
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
                    let copied = (|| {
                        if frames > SAMPLE_RATE {
                            return Err(io::Error::other("oversized microphone packet"));
                        }
                        // Flags alone do not tell us how many frames were lost. A forward
                        // device-position jump does, even when the driver omits the flag.
                        // The first packet has no reliable predecessor.
                        let gap = missing_frames(last_end, position);
                        if gap > u64::from(SAMPLE_RATE) {
                            return Err(io::Error::other(format!(
                                "microphone lost more than one second of audio (gap: {gap} frames; flags: 0x{flags:x}); recording incomplete"
                            )));
                        }
                        if last_end.is_some()
                            && (gap != 0
                                || flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0)
                        {
                            discontinuities += 1;
                        }
                        concealed_frames += gap;
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
                    let copied = copied?;
                    let gap = missing_frames(last_end, position);
                    last_end = position.checked_add(u64::from(frames));
                    // Emit silence in bounded chunks, not a gap-sized allocation. Keep both
                    // realtime output and the fallback WAV on the same capture timeline.
                    let silence = [0u8; 3200];
                    let mut remaining = gap;
                    while remaining != 0 {
                        let count = remaining.min(u64::from(SAMPLE_RATE / 10));
                        append_pcm(
                            &silence[..count as usize * 2],
                            &mut total,
                            &mut pending,
                            stop,
                            output,
                        )?;
                        remaining -= count;
                    }
                    append_pcm(&copied, &mut total, &mut pending, stop, output)?;
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
                "WASAPI capture stopped (elapsed: {} ms; captured: {} frames; packets: {packets}; discontinuities: {discontinuities}; silence inserted: {concealed_frames} frames; event timeouts: {timeouts}; max packet interval: {max_packet_gap_ms} ms)",
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
    fn discontinuity_positions_only_conceal_forward_gaps() {
        assert_eq!(missing_frames(None, 64_320), 0);
        assert_eq!(missing_frames(Some(64_000), 64_320), 320);
        assert_eq!(missing_frames(Some(64_000), 64_000), 0);
        assert_eq!(missing_frames(Some(64_000), 63_840), 0);
    }

    #[test]
    fn silence_keeps_pcm_in_order_and_respects_quota() {
        let stop = AtomicBool::new(false);
        let mut total = 0;
        let mut pending = Vec::new();
        let mut emitted = Vec::new();
        let mut output = |pcm: &[u8]| {
            emitted.extend_from_slice(pcm);
            Ok(())
        };
        append_pcm(&[1, 0], &mut total, &mut pending, &stop, &mut output).unwrap();
        append_pcm(&[0; 320], &mut total, &mut pending, &stop, &mut output).unwrap();
        append_pcm(&[2, 0], &mut total, &mut pending, &stop, &mut output).unwrap();
        assert_eq!(total, 324);
        assert_eq!(&pending[..2], &[1, 0]);
        assert!(pending[2..322].iter().all(|&byte| byte == 0));
        assert_eq!(&pending[322..], &[2, 0]);

        total = MAX_AUDIO_BYTES;
        append_pcm(&[3, 0], &mut total, &mut pending, &stop, &mut output).unwrap();
        assert!(stop.load(Ordering::Acquire));
        assert_eq!(total, MAX_AUDIO_BYTES);
        assert_eq!(pending.len(), 324);
        assert!(emitted.is_empty());
    }
}
