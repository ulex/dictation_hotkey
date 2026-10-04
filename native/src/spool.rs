//! Incremental WAV spool. The spool is finalized only after capture/processing have drained.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const SAMPLE_RATE: u32 = 16_000;
pub const MAX_SECONDS: u64 = 60 * 60;
pub const MAX_AUDIO_BYTES: u64 = MAX_SECONDS * SAMPLE_RATE as u64 * 2;
const HEADER_LEN: u64 = 44;

fn header(bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&1u16.to_le_bytes());
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&bytes.to_le_bytes());
    h
}

pub struct Spool {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
}

impl Spool {
    pub fn create(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        // The application's single-instance/session owner guarantees no other live spool.
        // Identifiable files here therefore came from a crashed prior run.
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                let recognizable = path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    s.len() == 39
                        && s.starts_with("dh-")
                        && s.ends_with(".wav")
                        && s.as_bytes()[3..35].iter().all(u8::is_ascii_hexdigit)
                });
                if recognizable {
                    fs::remove_file(path).map_err(|_| {
                        io::Error::other("unable to clean up a prior temporary recording")
                    })?;
                }
            }
        }
        // The kernel supplies cryptographically random bytes. Exclusive creation avoids collisions;
        // storing under per-user LOCALAPPDATA prevents the recording from roaming.
        for _ in 0..8 {
            let name = random_name()?;
            let path = directory.join(format!("dh-{name}.wav"));
            let mut options = OpenOptions::new();
            options.write(true).read(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(0);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(&header(0)) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(e);
                    }
                    return Ok(Self {
                        path,
                        file: Some(file),
                        bytes: 0,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot create spool",
        ))
    }

    pub fn append(&mut self, pcm: &[u8]) -> io::Result<()> {
        if !pcm.len().is_multiple_of(2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "PCM16 requires complete samples",
            ));
        }
        let next = self
            .bytes
            .checked_add(pcm.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "recording too large"))?;
        if next > MAX_AUDIO_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "one-hour recording quota reached",
            ));
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "spool already closed"))?
            .write_all(pcm)?;
        self.bytes = next;
        Ok(())
    }

    pub fn finish(&mut self) -> io::Result<&Path> {
        if let Some(file) = self.file.as_mut() {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header(self.bytes as u32))?;
            file.sync_all()?;
            self.file.take(); // release exclusive share lock before batch opens for reading
        }
        Ok(&self.path)
    }

    pub fn len(&self) -> u64 {
        self.bytes + HEADER_LEN
    }

    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
}

pub fn random_name() -> io::Result<String> {
    let mut token = [0u8; 16];
    random_token(&mut token)?;
    Ok(token.iter().map(|b| format!("{b:02x}")).collect())
}

// On Windows use the system's secure random generator (bcrypt, no bundled library).
#[cfg(windows)]
fn random_token(out: &mut [u8; 16]) -> io::Result<()> {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(algorithm: isize, buffer: *mut u8, count: u32, flags: u32) -> i32;
    }
    if unsafe { BCryptGenRandom(0, out.as_mut_ptr(), out.len() as u32, 2) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(not(windows))]
fn random_token(out: &mut [u8; 16]) -> io::Result<()> {
    use io::Read;
    File::open("/dev/urandom")?.read_exact(out)
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_and_cleanup() {
        let dir = std::env::temp_dir().join(format!("dh-spool-test-{}", std::process::id()));
        let path;
        {
            let mut spool = Spool::create(&dir).unwrap();
            assert!(spool.append(&[1]).is_err());
            spool.append(&[0x34, 0x12, 0, 0]).unwrap();
            path = spool.finish().unwrap().to_owned();
            assert!(spool.append(&[0, 0]).is_err());
            assert_eq!(spool.finish().unwrap(), path);
            let wav = fs::read(&path).unwrap();
            assert_eq!(wav.len(), 48);
            assert_eq!(&wav[40..44], &4u32.to_le_bytes());
            assert_eq!(&wav[44..46], &[0x34, 0x12]);
        }
        assert!(!path.exists());
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn crash_cleanup_only_identifiable_files() {
        let dir = std::env::temp_dir().join(format!("dh-cleanup-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("dh-0123456789abcdef0123456789abcdef.wav");
        fs::write(&stale, b"audio fixture").unwrap();
        fs::write(dir.join("other.wav"), b"not ours").unwrap();
        // A non-ASCII filename with a similar byte length must never panic or be removed.
        fs::write(dir.join("dh-😀😀😀😀😀😀😀😀.wav"), b"not ours").unwrap();
        let spool = Spool::create(&dir).unwrap();
        assert!(!stale.exists());
        assert!(dir.join("other.wav").exists());
        drop(spool);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "writes a full simulated hour (115.2 MB) to disk"]
    fn long_recording_has_no_full_session_ram_buffer() {
        let dir = std::env::temp_dir().join(format!("dh-hour-test-{}", std::process::id()));
        let mut spool = Spool::create(&dir).unwrap();
        let chunk = [0u8; 3200];
        for _ in 0..36000 {
            spool.append(&chunk).unwrap();
        }
        assert_eq!(spool.len(), MAX_AUDIO_BYTES + 44);
        assert!(spool.append(&[0, 0]).is_err());
        let path = spool.finish().unwrap().to_owned();
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_AUDIO_BYTES + 44);
        let mut header = [0u8; 44];
        use io::Read;
        File::open(&path).unwrap().read_exact(&mut header).unwrap();
        assert_eq!(
            u32::from_le_bytes(header[40..44].try_into().unwrap()) as u64,
            MAX_AUDIO_BYTES
        );
        drop(spool);
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn quota_never_grows_past_limit() {
        let dir = std::env::temp_dir().join(format!("dh-spool-quota-{}", std::process::id()));
        let mut spool = Spool::create(&dir).unwrap();
        spool.bytes = MAX_AUDIO_BYTES;
        assert!(spool.append(&[0, 0]).is_err());
        drop(spool);
        fs::remove_dir(dir).unwrap();
    }
}
