# 202020

Windows tray app for the 20-20-20 rule: every 20 minutes, a fullscreen overlay asks you to look away for 20 seconds. Performance and low CPU and memory overhead are the top priority.

## Capsule bar

The capsule bar replaces the Windows 11 taskbar with a floating capsule you can drag anywhere, holding Start, pinned programs, open windows, tray icons, keyboard layout, volume and clock. Hold Shift and scroll over it to resize it, and middle-click an app to open another instance.

## Launcher

`Alt+Space` opens a fuzzy search over apps, open windows, commands and files.

## CapsLock hyper

Tap CapsLock for `Esc`, or hold it with another key for navigation, editing and window shortcuts, which are listed under `[hyper]` in `config.toml`.

## Desktops

Tap ``Win+` `` to switch between two desktops, or hold it to send the active window to the other one. Hidden windows come back when 202020 quits, crashes or is killed.

## Keep awake

The tray menu's Keep awake submenu stops the PC from sleeping and the screen from turning off for 30 minutes to 8 hours, or until turned off, and shows the time left the next time the menu opens.

## Running

`202020.exe` is portable: `config.toml` and logs live next to it, so put it in a writable folder with `capsule_hook.dll`. To start at login, put a shortcut in `shell:startup`. `--safe` keeps the native taskbar visible.

`config.toml` is created on first run, and its comments document the settings. To report a problem, set `level = "debug"` under `[logging]`, restart, and attach `logs\log.txt`.

## Build

On Windows:

```sh
cargo build --release -p twenty-twenty-twenty -p capsule-hook
```

Cross-compiling from Linux:

```sh
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
scripts/build.sh      # target/x86_64-pc-windows-msvc/release/{202020.exe,capsule_hook.dll}
scripts/check.sh      # fmt, clippy, core tests, build and artifact checks
```
