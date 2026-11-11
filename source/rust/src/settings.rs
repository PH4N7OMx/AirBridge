use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::*;

#[derive(Clone, Debug)]
pub struct Settings {
    pub path: PathBuf,
    values: BTreeMap<String, String>,
}
impl Settings {
    pub fn load(path: PathBuf) -> io::Result<Self> {
        let text = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut values = BTreeMap::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.starts_with(['#', ';', '[']) {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                values.insert(k.trim().into(), v.trim().into());
            }
        }
        Ok(Self { path, values })
    }
    pub fn name(&self) -> String {
        let name = self
            .values
            .get("receiver_name")
            .map(String::as_str)
            .unwrap_or("AirBridge");
        if name.is_empty() || name.len() > 50 || name.contains(['.', '\\', '\0', '\r', '\n']) {
            "AirBridge".into()
        } else {
            name.into()
        }
    }
    pub fn number(&self, key: &str, default: u32, min: u32, max: u32) -> u32 {
        self.values
            .get(key)
            .and_then(|s| s.parse().ok())
            .filter(|n| (min..=max).contains(n))
            .unwrap_or(default)
    }
    pub fn flag(&self, key: &str, default: bool) -> bool {
        self.values.get(key).map(|s| s == "1").unwrap_or(default)
    }
    pub fn signed(&self, key: &str) -> Option<i32> {
        self.values.get(key)?.parse().ok()
    }
    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    pub fn set(&mut self, k: &str, v: impl ToString) {
        self.values.insert(k.into(), v.to_string());
    }
    pub fn theme_mode(&self) -> u8 {
        match self.values.get("theme_mode").map(String::as_str) {
            Some("light") => 1,
            Some("dark") => 2,
            _ => 0,
        }
    }
    pub fn manual_resolution(&self) -> (u32, u32) {
        let size = (
            self.number("mirror_width", 1920, 320, 8192),
            self.number("mirror_height", 1080, 320, 8192),
        );
        // The initial port wrote this square as a default, before exposing Auto.
        if self.auto_resolution() && size == (2622, 2622) {
            (1920, 1080)
        } else {
            size
        }
    }
    pub fn auto_resolution(&self) -> bool {
        self.flag("mirror_auto_resolution", true)
    }
    pub fn resolution(&self) -> (u32, u32) {
        // A generous square works for both orientations without claiming to know
        // the sender's native screen. AirPlay chooses the actual stream size.
        if self.auto_resolution() {
            (4096, 4096)
        } else {
            self.manual_resolution()
        }
    }
    pub fn save(&self) -> io::Result<()> {
        let text = self
            .values
            .iter()
            .map(|(k, v)| format!("{k}={v}\n"))
            .collect::<String>();
        atomic_write(&self.path, text.as_bytes())
    }
}
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| {
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        let from = crate::platform::wide(tmp.as_os_str());
        let to = crate::platform::wide(path.as_os_str());
        // Replace atomically; keep the old settings intact if the commit fails.
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

pub fn identity(path: &Path) -> io::Result<[u8; 32]> {
    match fs::read(path) {
        Ok(data) => data.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid receiver identity; original file preserved",
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut seed = [0; 32];
            getrandom::getrandom(&mut seed).map_err(|e| io::Error::other(e.to_string()))?;
            atomic_write(path, &seed)?;
            Ok(seed)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_defaults_replace_only_the_legacy_square() {
        let mut s = Settings {
            path: PathBuf::new(),
            values: BTreeMap::new(),
        };
        assert_eq!(s.theme_mode(), 0);
        assert_eq!(s.manual_resolution(), (1920, 1080));
        s.set("mirror_width", 2622);
        s.set("mirror_height", 2622);
        assert_eq!(s.manual_resolution(), (1920, 1080));
        assert_eq!(s.resolution(), (4096, 4096));
        s.set("mirror_auto_resolution", 0);
        assert_eq!(s.resolution(), (2622, 2622));
    }
    #[test]
    fn automatic_resolution_defaults_on_and_preserves_manual_limits() {
        let mut s = Settings {
            path: PathBuf::new(),
            values: BTreeMap::new(),
        };
        s.set("mirror_width", 1440);
        s.set("mirror_height", 3200);
        assert!(s.auto_resolution());
        assert_eq!(s.resolution(), (4096, 4096));
        s.set("mirror_auto_resolution", "0");
        assert_eq!(s.resolution(), (1440, 3200));
        s.set("mirror_auto_resolution", "1");
        assert_eq!(s.resolution(), (4096, 4096));
        assert_eq!(s.values.get("mirror_width").unwrap(), "1440");
        assert!(u64::from(s.resolution().0) * u64::from(s.resolution().1) <= 32 * 1024 * 1024);
    }
    #[test]
    fn settings_preserve_unknown_keys_and_validate_numbers() {
        let path =
            std::env::temp_dir().join(format!("airbridge-settings-{}.ini", std::process::id()));
        fs::write(
            &path,
            "\u{feff}receiver_name=Тест\nmax_fps=-1\nfuture_key=hello=world\n",
        )
        .unwrap();
        let s = Settings::load(path.clone()).unwrap();
        assert_eq!(s.name(), "Тест");
        assert_eq!(s.number("max_fps", 60, 1, 60), 60);
        s.save().unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("future_key=hello=world")
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn corrupt_identity_is_never_silently_replaced() {
        let path = std::env::temp_dir().join(format!("airbridge-identity-{}", std::process::id()));
        fs::write(&path, b"bad").unwrap();
        assert!(identity(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"bad");
        fs::remove_file(path).unwrap();
    }
}
