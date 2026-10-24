# 202020

Windows tray app for the 20-20-20 rule: every 20 minutes, a fullscreen overlay on the primary monitor asks you to look away for 20 seconds.

- Tray icon shows minutes left, a 10-second countdown before a break, or a pause symbol.
- Click the tray icon for Pause/Resume, Break now, Reset timer, Quit.
- `Win+Alt+C` toggles pause.
- `Esc` or `Space` ends a break early.
- The timer resets after the system sleeps.

## Build

```
cargo build --release
```

Produces `target\release\202020.exe`. To start at login, put a shortcut to it in `shell:startup`.
