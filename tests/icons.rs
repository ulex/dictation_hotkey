//! Resource checks run without touching the Windows desktop.
#[test]
fn icons_contain_native_size_png_frames() {
    for data in [
        include_bytes!("../resources/tray-idle.ico").as_slice(),
        include_bytes!("../resources/tray-recording.ico").as_slice(),
        include_bytes!("../resources/tray-processing.ico").as_slice(),
    ] {
        assert_eq!(&data[..6], &[0, 0, 1, 0, 7, 0]);
        assert!(data.len() < 8 * 1024, "keep status icons compact");
        let mut previous_end = 6 + 7 * 16;
        for (index, size) in [16u32, 20, 24, 32, 40, 48, 64].into_iter().enumerate() {
            let entry = &data[6 + index * 16..6 + (index + 1) * 16];
            let dimension = if size == 256 { 0 } else { size as u8 };
            assert_eq!(&entry[..4], &[dimension, dimension, 0, 0]);
            assert_eq!(&entry[4..8], &[1, 0, 32, 0]);
            let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            assert_eq!(start, previous_end);
            let frame = &data[start..start + len];
            assert_eq!(&frame[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(&frame[12..16], b"IHDR");
            assert_eq!(u32::from_be_bytes(frame[16..20].try_into().unwrap()), size);
            assert_eq!(u32::from_be_bytes(frame[20..24].try_into().unwrap()), size);
            assert_eq!(&frame[24..26], &[8, 6], "8-bit RGBA with transparency");
            previous_end = start + len;
        }
        assert_eq!(previous_end, data.len());
    }
}
