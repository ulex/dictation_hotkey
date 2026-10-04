use serde_json::{Map, Value};
use std::{fs, io, path::Path};

pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_FIELD_CHARS: usize = 4096;

/// Preserve unknown fields, including settings migrated from the legacy application.
#[derive(Clone)]
pub struct Config {
    fields: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        let fields = serde_json::from_str::<Value>(
            r#"{"api_key":"","hotkey_copilot":false,"hotkey_win_h":true,"hotkey_custom":"","language":"","typing_mode":"paste","paste_shortcut":"shift_insert","start_with_windows":false,"offline_mode":false,"model":"","offline_model":"","base_url":""}"#,
        )
        .expect("static JSON")
        .as_object()
        .expect("static object")
        .clone();
        Self { fields }
    }
}

impl Config {
    pub fn from_bytes(input: &[u8]) -> io::Result<Self> {
        if input.len() as u64 > MAX_CONFIG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "configuration exceeds 64 KiB",
            ));
        }
        let object = serde_json::from_slice::<Value>(input)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "configuration is not an object")
            })?;
        let mut config = Self::default();
        for (name, value) in object {
            let valid = match config.fields.get(&name) {
                Some(Value::String(_)) => value
                    .as_str()
                    .is_some_and(|s| s.chars().count() <= MAX_FIELD_CHARS),
                Some(Value::Bool(_)) => value.is_boolean(),
                _ => true,
            };
            if valid {
                config.fields.insert(name, value);
            }
        }
        Ok(config)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() > MAX_CONFIG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "configuration exceeds 64 KiB",
            ));
        }
        let mut bytes = Vec::new();
        use io::Read;
        file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?;
        fs::create_dir_all(parent)?;
        let mut fields = self.fields.clone();
        fields.insert("schema_version".into(), Value::from(1));
        let bytes = serde_json::to_vec_pretty(&fields)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "configuration exceeds 64 KiB",
            ));
        }
        // Preserve the first legacy configuration before native changes. Never overwrite it.
        if path.exists() {
            let backup = parent.join("config.pre-native.json");
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut dest) => {
                    use io::{Read, Write};
                    let result = (|| {
                        let mut data = Vec::new();
                        fs::File::open(path)?
                            .take(MAX_CONFIG_BYTES + 1)
                            .read_to_end(&mut data)?;
                        if data.len() as u64 > MAX_CONFIG_BYTES {
                            return Err(io::Error::other(
                                "existing configuration exceeds backup limit",
                            ));
                        }
                        dest.write_all(&data)?;
                        dest.sync_all()
                    })();
                    drop(dest);
                    if let Err(e) = result {
                        let _ = fs::remove_file(backup);
                        return Err(e);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e),
            }
        }
        // create_new prevents clobbering a different process's pending save; rename is atomic
        // on a single volume. No existing config is removed on a failed write.
        let mut i = 0;
        loop {
            let temp = parent.join(format!(".config-{}-{i}.tmp", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
            {
                Ok(mut file) => {
                    use io::Write;
                    let result = (|| {
                        file.write_all(&bytes)?;
                        file.sync_all()?;
                        drop(file);
                        replace_file(&temp, path)
                    })();
                    if result.is_err() {
                        let _ = fs::remove_file(&temp);
                    }
                    return result;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && i < 9 => i += 1,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        let custom = self.string("hotkey_custom");
        if !custom.is_empty() && crate::hotkey::parse(custom).is_none() {
            return Err(io::Error::other(
                "invalid custom hotkey (use modifiers + letter, number, or F1-F24)",
            ));
        }
        let reserved = [
            (self.boolean("hotkey_win_h"), "Win+H"),
            (self.boolean("hotkey_copilot"), "Win+C"),
            (self.boolean("hotkey_copilot"), "Win+Shift+F23"),
        ];
        if !custom.is_empty()
            && reserved
                .iter()
                .any(|(on, key)| *on && crate::hotkey::parse(custom) == crate::hotkey::parse(key))
        {
            return Err(io::Error::other(
                "custom hotkey duplicates an enabled built-in shortcut",
            ));
        }
        if !matches!(self.string("typing_mode"), "paste" | "keystrokes") {
            return Err(io::Error::other("invalid typing mode"));
        }
        if !matches!(
            self.string("paste_shortcut"),
            "shift_insert" | "ctrl_v" | "ctrl_shift_v"
        ) {
            return Err(io::Error::other("invalid paste shortcut"));
        }
        for key in ["model", "offline_model"] {
            if !self.string(key).is_empty() && !crate::wire::valid_model(self.string(key)) {
                return Err(io::Error::other("invalid model identifier"));
            }
        }
        if !self.string("base_url").is_empty() {
            crate::wire::endpoint(self.string("base_url"), crate::wire::DEFAULT_MODEL)?;
        }
        if self.string("api_key").chars().any(char::is_control) {
            return Err(io::Error::other("invalid API key"));
        }
        Ok(())
    }

    pub fn string(&self, name: &str) -> &str {
        self.fields.get(name).and_then(Value::as_str).unwrap_or("")
    }
    pub fn boolean(&self, name: &str) -> bool {
        self.fields
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.fields.insert(name.to_owned(), Value::Bool(value));
    }
    pub fn set_string(&mut self, name: &str, value: &str) -> io::Result<()> {
        if value.chars().count() > MAX_FIELD_CHARS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "setting exceeds limit",
            ));
        }
        self.fields
            .insert(name.to_owned(), Value::String(value.to_owned()));
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp, path)
}

#[cfg(windows)]
fn replace_file(temp: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_legacy_roundtrip() {
        let cfg = Config::from_bytes(
            br#"{"api_key":"secret","extra":{"a":1},"hotkey_win_h":"bad","offline_mode":true}"#,
        )
        .unwrap();
        assert!(cfg.boolean("hotkey_win_h"));
        assert!(cfg.boolean("offline_mode"));
        assert_eq!(cfg.string("api_key"), "secret");
        let path = std::env::temp_dir()
            .join(format!("dh-config-{}", std::process::id()))
            .join("config.json");
        cfg.save(&path).unwrap();
        cfg.save(&path).unwrap(); // Windows replacement of an existing config
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["extra"]["a"], 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn validation_and_backup() {
        let mut cfg = Config::default();
        assert!(cfg.validate().is_ok());
        cfg.set_string("hotkey_custom", "Win+H").unwrap();
        assert!(cfg.validate().is_err());
        cfg.set_string("hotkey_custom", "Ctrl+Alt+F12").unwrap();
        cfg.set_string("base_url", "http://insecure.example")
            .unwrap();
        assert!(cfg.validate().is_err());
        cfg.set_string("base_url", "").unwrap();
        assert!(cfg.validate().is_ok());
        let directory = std::env::temp_dir().join(format!("dh-backup-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let original = br#"{"api_key":"legacy-fixture","unknown":1}"#;
        fs::write(&path, original).unwrap();
        cfg.save(&path).unwrap();
        cfg.save(&path).unwrap();
        assert_eq!(
            fs::read(directory.join("config.pre-native.json")).unwrap(),
            original
        );
        assert_eq!(
            Config::load(&path).unwrap().string("hotkey_custom"),
            "Ctrl+Alt+F12"
        );
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn save_failure_preserves_old_file() {
        let directory = std::env::temp_dir().join(format!("dh-save-fail-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        let original = b"old-file";
        fs::write(&path, original).unwrap();
        for i in 0..10 {
            fs::write(
                directory.join(format!(".config-{}-{i}.tmp", std::process::id())),
                b"busy",
            )
            .unwrap();
        }
        assert!(Config::default().save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn reject_oversized_and_malformed() {
        assert!(Config::from_bytes(&vec![b'x'; MAX_CONFIG_BYTES as usize + 1]).is_err());
        assert!(Config::from_bytes(b"[]").is_err());
        assert!(Config::from_bytes(b"{").is_err());
        assert_eq!(
            Config::from_bytes(format!(r#"{{"api_key":"{}"}}"#, "x".repeat(5000)).as_bytes())
                .unwrap()
                .string("api_key"),
            ""
        );
    }
}
