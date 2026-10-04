#![cfg(windows)]
//! Opt-in hardware/network probes, separate from desktop-only tests.
use dictation_hotkey_native::{audio, service, service_ws, spool::Spool, wire};
use std::{
    sync::{atomic::AtomicBool, mpsc::sync_channel, Arc},
    time::{Duration, Instant},
};

#[test]
#[ignore = "opens the default microphone for two seconds; audio is counted then discarded"]
fn wasapi_pcm16_capture_and_stop() {
    let stop = Arc::new(AtomicBool::new(false));
    let timer_stop = stop.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        timer_stop.store(true, std::sync::atomic::Ordering::Release);
    });
    let started = Instant::now();
    let mut bytes = 0usize;
    let mut chunks = 0;
    let captured = audio::capture(&stop, |pcm| {
        assert!(!pcm.is_empty());
        assert_eq!(pcm.len() % 2, 0, "PCM16 sample alignment");
        assert!(pcm.len() <= 3200, "at most 100 ms per chunk");
        bytes += pcm.len();
        chunks += 1;
        Ok(())
    });
    timer.join().unwrap();
    captured.expect("default microphone must support WASAPI capture");
    assert!(bytes >= 16000, "expected at least half a second of PCM");
    assert!(started.elapsed() < Duration::from_secs(5));
    eprintln!(
        "WASAPI: {bytes} bytes, {chunks} chunks, {:?}",
        started.elapsed()
    );
}

#[test]
#[ignore = "contacts Mistral with a deliberately invalid key; uploads only synthetic silence"]
fn winhttp_realtime_and_batch_authentication_rejection() {
    let invalid_key = "dictation-windows-validation-not-a-real-key";
    let (_tx, rx) = sync_channel(1);
    let error = service_ws::realtime(
        invalid_key,
        wire::DEFAULT_MODEL,
        wire::DEFAULT_URL,
        rx,
        Arc::new(AtomicBool::new(false)),
        |_| panic!("invalid credential must never produce text"),
    )
    .unwrap_err();
    eprintln!("Realtime rejection: {error}");
    assert!(error.to_string().contains("HTTP 401"), "{error}");

    let mut spool = Spool::create(&std::env::temp_dir()).unwrap();
    spool.append(&[0; 3200]).unwrap();
    let error = service::batch(
        spool.finish().unwrap(),
        wire::DEFAULT_BATCH_MODEL,
        invalid_key,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    eprintln!("Batch rejection: {error}");
    assert!(error.to_string().contains("HTTP 401"), "{error}");
}

#[test]
fn cancelled_transports_return_promptly() {
    let cancelled = Arc::new(AtomicBool::new(true));
    let started = Instant::now();
    let (_tx, rx) = sync_channel(1);
    assert!(service_ws::realtime(
        "test-key",
        wire::DEFAULT_MODEL,
        "wss://127.0.0.1:9/v1/audio/transcriptions/realtime",
        rx,
        cancelled.clone(),
        |_| Ok(()),
    )
    .is_err());
    assert!(service::batch(
        std::path::Path::new("nonexistent.wav"),
        wire::DEFAULT_BATCH_MODEL,
        "test-key",
        cancelled,
    )
    .is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
