use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, SetUnhandledExceptionFilter,
};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::PCWSTR;

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

fn locate(addr: usize) -> (String, usize) {
    let mut module = HMODULE::default();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, PCWSTR(addr as *const u16), &mut module) }.is_err() {
        return ("?".into(), addr);
    }
    let mut buf = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    let path = String::from_utf16_lossy(&buf[..n]);
    let name = path.rsplit('\\').next().unwrap_or_default().to_owned();
    (name, addr.wrapping_sub(module.0 as usize))
}

unsafe extern "system" fn log_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    let record = unsafe { &*(*info).ExceptionRecord };
    let (module, offset) = locate(record.ExceptionAddress as usize);
    log::error!("crash: exception {:#010x} at {module}+{offset:#x}", record.ExceptionCode.0 as u32);
    log::logger().flush();
    EXCEPTION_CONTINUE_SEARCH
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
    if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file) })).is_err() {
        return;
    }
    log::set_max_level(level);
    log::info!("202020 {} pid {}", env!("CARGO_PKG_VERSION"), std::process::id());
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        log::logger().flush();
    }));
    unsafe {
        SetUnhandledExceptionFilter(Some(log_crash));
    }
}
