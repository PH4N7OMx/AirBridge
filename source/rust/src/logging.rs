use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct FileLogger {
    path: PathBuf,
    gate: Mutex<()>,
}
impl FileLogger {
    pub fn install(path: PathBuf) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(&path)?;
        log::set_boxed_logger(Box::new(Self {
            path,
            gate: Mutex::new(()),
        }))
        .map_err(io::Error::other)?;
        log::set_max_level(log::LevelFilter::Info);
        Ok(())
    }
}
impl log::Log for FileLogger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let _guard = self.gate.lock().unwrap_or_else(|p| p.into_inner());
        // Logging is best effort: a full disk must not take down the receiver.
        let _ = (|| -> io::Result<()> {
            if fs::metadata(&self.path).is_ok_and(|m| m.len() >= 8 * 1024 * 1024) {
                let previous = self.path.with_extension("previous.log");
                if previous.exists() {
                    fs::remove_file(&previous)?;
                }
                fs::rename(&self.path, previous)?;
            }
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            writeln!(file, "{stamp} {} {}", r.level(), r.args())
        })();
    }
    fn flush(&self) {}
}
