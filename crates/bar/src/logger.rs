use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use windows::Win32::System::SystemInformation::GetLocalTime;

struct FileLogger {
    file: Mutex<File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let t = unsafe { GetLocalTime() };
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(
                f,
                "{:02}:{:02}:{:02}.{:03} {:5} {}",
                t.wHour,
                t.wMinute,
                t.wSecond,
                t.wMilliseconds,
                record.level(),
                record.args()
            );
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

const MAX_LOG_BYTES: u64 = 1 << 20;

pub fn init(dir: &Path, name: &str, level: log::LevelFilter) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("old.txt"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file) })).is_ok() {
        log::set_max_level(level);
    }
}
