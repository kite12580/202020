# 202020

Windows tray app for the 20-20-20 rule: every 20 minutes, a fullscreen overlay on the primary monitor asks you to look away for 20 seconds. It can also replace the Windows 11 taskbar with the capsule bar.

- Tray icon shows minutes left, a 10-second countdown before a break, or a pause symbol.
- Click the tray icon for Capsule bar, Pause/Resume, Break now, Reset timer, Edit config, Reload config, Quit.
- `Win+Alt+C` toggles pause.
- `Esc` or `Space` ends a break early.
- The timer resets after the system sleeps.

## Capsule bar

A floating, translucent, draggable capsule that replaces the Windows 11 taskbar. The native taskbar is hidden, so maximized windows fill the whole screen. Turn it on or off with **Capsule bar** in the tray menu. While it is off, it uses no threads, hooks, or extra DLLs.

```
╭──────────────────────────────────────╮
│ ⊞ C  C  V [T] E  ^ W Q 中 <) 2:32 PM │
│                             11/01/26 │
╰──────────────────────────────────────╯
```

Hold the left button anywhere on the capsule and drag to move it (the position is remembered). It snaps to the left edge, the center, the right edge and the bottom of the screen. Placed against the left or right edge, it stays there as icons come and go. Clicks without dragging work as usual.

| Area | Action |
|---|---|
| Start | Left: open Start; right: the Win+X menu |
| Window icons | Programs pinned to the native taskbar come first, in its pin order, and always show: one icon per open window, or the program's icon when none is open. Other windows follow in the order they were opened. A short bar under an icon marks an open window; a longer accent-colored bar marks the active one. Left: activate, or minimize if already in front (on a closed pinned program: launch it); middle: close; right: system menu; hover: title |
| `Win+1` … `Win+0` | Same as the native taskbar (its order, pinned programs included). The bar briefly wakes the hidden native taskbar and hands it the keystroke |
| `^` | Shows all tray icons. Right-click an icon in the popup to pin it to the capsule or unpin it |
| Pinned tray icons | Click, double-click and right-click behave as on the native taskbar |
| Keyboard layout | Shows the current layout (中 / EN …); shows 英 when a Chinese IME is in English mode. Left: next layout; right: list all layouts |
| Volume | Wheel adjusts volume; click opens Quick Settings (like `Win+A`); hover shows the percentage |
| Clock | Time on top, date (month/day/year) below. Click opens notifications and calendar (like `Win+N`); hover shows the full date and weekday |

Right-click the clock or an empty spot for Restart capsule bar and Quit capsule bar (turns the bar off).

The capsule hides while a foreground program is fullscreen (games, F11, fullscreen video, slideshows) and during breaks.

Latest Windows 11 only, single monitor.

While the bar is running, a `--watchdog` child process restores the taskbar's auto-hide state, visibility and work area if 202020 exits unexpectedly. "End process tree" in Task Manager kills the watchdog too; in that case run `202020.exe --restore`, or turn off "Automatically hide the taskbar" in Settings.

### Known limitations

- Windows 11 system tray icons (network, battery, system volume) don't go through `Shell_NotifyIcon`, so they aren't shown.
- Tray icons are re-registered through the `TaskbarCreated` broadcast. Programs started before the bar that ignore this broadcast need a restart for their icons to appear.
- Tray icons of programs running as administrator can only be clicked if 202020 also runs as administrator; otherwise a hint is shown.
- Two input methods of the same language (e.g. Pinyin and Wubi) show the same name in the layout menu.
- The wheel only adjusts volume when "Scroll inactive windows" is on (the Windows default).

## Running

`202020.exe` is portable: `config.toml` (and logs, if enabled) live next to the exe, so put it in a writable folder. Keep `capsule_hook.dll` next to it: while the capsule bar runs, it is loaded into explorer to pass tray icons to the bar. To start at login, put a shortcut to it in `shell:startup`.

| Flag | Effect |
|---|---|
| `--safe` | Don't hide the native taskbar (useful for a first test of the capsule bar) |
| `--verbose` | Write debug logs to `logs\log.txt` |
| `--diag` | Like `--verbose`, plus why each window was shown or filtered |
| `--restore` | Restore the native taskbar (after both 202020 and its watchdog were killed) |

## Config

`config.toml` next to the exe is created on first run. Capsule bar settings live under `[capsule_bar]`. While the bar is running, changes apply on save; `enabled` is read at startup and set by the tray menu. See the comments in the file. If acrylic stops working (e.g. the capsule stays a solid color), set `backdrop` to `"accent"`.

## Build

On Windows:

```
cargo build --release -p twenty-twenty-twenty -p capsule-hook
```

Produces `target\release\202020.exe` and `target\release\capsule_hook.dll`.

Cross-compiling from Linux:

```sh
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
scripts/build.sh      # target/x86_64-pc-windows-msvc/release/{202020.exe,capsule_hook.dll}
scripts/check.sh      # fmt, clippy, core tests, build and artifact checks
```

The exe links the CRT statically and doesn't need the VC++ runtime.
