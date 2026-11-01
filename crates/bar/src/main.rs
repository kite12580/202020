#![windows_subsystem = "windows"]

mod app;
mod config_store;
mod gfx;
mod icons;
mod keyboard;
mod keyhook;
mod logger;
mod menu;
mod pinned;
mod popup;
mod shell;
mod theme;
mod tooltip;
mod tray;
mod util;
mod volume;
mod windows_list;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    match args.first().map(String::as_str) {
        Some("--watchdog") => {
            logger::init(&config_store::log_dir(), "watchdog.txt", log::LevelFilter::Info);
            std::process::exit(shell::run_watchdog(&args[1..]));
        }
        Some("--restore") => {
            logger::init(&config_store::log_dir(), "log.txt", log::LevelFilter::Info);
            shell::force_restore();
            return;
        }
        _ => {}
    }
    let diag = flag("--diag");
    let level = if diag || flag("--verbose") { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    logger::init(&config_store::log_dir(), "log.txt", level);
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        log::logger().flush();
    }));
    app::run(app::Options { safe: flag("--safe"), diag });
}
