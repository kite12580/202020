# 202020

Windows tray app for the 20-20-20 rule: every 20 minutes, a fullscreen overlay on the primary monitor asks you to look away for 20 seconds. It can also replace the Windows 11 taskbar with the capsule bar, comes with a Spotlight-style launcher on `Alt+Space`, and turns CapsLock into a hyper key.

- Tray icon shows minutes left, a 10-second countdown before a break, or a pause symbol.
- Click the tray icon for Capsule bar, Launcher, CapsLock hyper, Pause/Resume, Break now, Reset timer, Edit config, Reload config, Quit.
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

## Launcher

`Alt+Space` opens a search box in the upper middle of the screen; press it again, `Esc`, or click elsewhere to close it. It searches:

| Source | What | Prefix |
|---|---|---|
| Apps | Everything in the Start menu's All apps list, including Store apps (off by default) | |
| Windows | Open windows, by title or program name (off by default) | `@` |
| Commands | 202020 actions (pause, break now, capsule bar, config), Empty Recycle Bin, Settings, Task Manager, Control Panel, and your own commands | `>` |
| Files | Files and folders under the folders listed in the config (`.exe` and `.lnk` files in `C:\Games` by default) | `/` |

Matching is fuzzy: the letters you type must appear in order, and matches at word starts rank higher. Chinese names also match by 拼音, by 拼音首字母, or by a mix of both, with every reading of a 多音字 tried: `wx`, `weixin` and `wxin` all find 微信. Apps also match by their program name (`calc` finds 计算器). Things you open often rank higher. With nothing typed, the most-used entries are shown. Start with a prefix to search a single source.

| Key | Action |
|---|---|
| `Enter` / click | Open the app or file, switch to the window, or run the command |
| `↑` `↓` `Tab` / wheel | Move the selection |
| Right-click / `Shift+F10` | The item's Explorer context menu (Run as administrator, Open file location, Properties …); a window's system menu |

Each source can be turned off or weighted under `[launcher]` in the config, and the hotkey can be changed (applied by Reload config). If another program already uses the hotkey, the tray menu shows "Launcher (hotkey unavailable)". While the launcher is on, `Alt+Space` no longer opens a window's system menu.

The launcher keeps nothing in memory until you first open it. Its index is rebuilt in the background each time it opens and freed after 10 minutes without use. Explorer opens what you pick, so programs start as they would from the Start menu, without administrator rights even when 202020 has them. Launch counts are kept in memory and written to `launcher_history.tsv` next to the exe only when 202020 quits or Windows shuts down.

## CapsLock hyper

Tap CapsLock for `Esc`. Hold it and press another key for the shortcuts below; add the left `Alt` for the second column. CapsLock itself never toggles. Turn it on or off with **CapsLock hyper** in the tray menu (on by default); its settings under `[hyper]` apply on Reload config or `CapsLock+F5`.

These are the default keys. They are listed in the config under `[hyper.keys]` and `[hyper.alt_keys]`, where each key maps to keys to press (`"End Enter"`, `"Ctrl+Shift+Left"`) or an action; the comments there list the actions. Edit them and reload to remap without restarting. Without those tables the defaults apply; with them, keys left out do nothing, and entries 202020 can't read are skipped (logged at `warn`).

| Key | CapsLock + key | CapsLock + Alt + key |
|---|---|---|
| `H` `J` `K` `L` | ← ↓ ↑ → | Select ← ↓ ↑ → |
| `B` `E` | Word left / right | Select word left / right |
| `;` `'` | Home / End | Select to Home / End |
| `Enter` `Space` | End / → | |
| `G` | End of document | Start of document |
| `R` `Q` | Page up / down | Wheel up / down |
| `N` `M` | Previous / next tab | |
| `O` | New line below | New line above |
| `U` | Undo (terminals: `Ctrl+/`) | Redo |
| `Z` | Save | Save all (`Ctrl+Shift+S`) |
| `C` | `Alt+Tab` | |
| `V` | Clipboard history (`Ctrl+Shift+V`) | Next / previous virtual desktop, alternating |
| `/` | Find | |
| `1` `2` `3` | Left / middle / right mouse button (hold to drag) | |
| `,` `.` | Browser back / forward | |
| `` ` `` | Context menu key | |
| `W` | Delete word left (terminals, Vim: `Ctrl+W`) | Join with the next line |
| `Backspace` | Delete the word under the caret | |
| `D` | Delete line (terminals, Vim: `Ctrl+U`) | Clear line |
| `X` | Cut (copy, then `Del`) | Copy, then `Backspace` |
| `Y` | Copy the selection, or the line if nothing is selected | Same |
| `P` | Paste | Paste as plain text |
| `9` | Wrap the selection in `( )` | Wrap in `" "` |
| `0` | Type `)` | Wrap in `' '` |
| `[` | ` {` + new line + `}` at the end of the line | Wrap in `[ ]` |
| `=` | Wrap in ` + "` … `" + ` | |
| `Tab` | Expand the hotstring before the caret | |
| `F` | | `Esc`; with the mouse at the screen's left edge, move the window: top → small, middle → large, bottom → bottom-right corner and always on top |
| `F1` | Toggle always on top | Toggle the title bar |
| `F2` `F3` | — / next track | Play-pause / previous track |
| `F4` | Tap: toggle transparency. Hold and turn the wheel: adjust it | |
| `F5` | Reload config | |
| `F9` | Power Options | |

- While CapsLock and the left `Alt` are held, the mouse moves at `slow_mouse_speed`.
- With `Ctrl`, `Shift` or `Win` also held, keys go through unchanged.
- "Terminals" are the programs listed in `terminals`; "Vim" is gvim and nvim-qt.
- Hotstrings live under `[hyper.hotstrings]`: type a key such as `sig`, press `CapsLock+Tab`, and the key is replaced by its text.
- Copying, wrapping, plain paste, hotstrings and line deletion go through the clipboard and put back what was there. What 202020 writes is kept out of clipboard history (Win+V, Ditto), but copies the program itself makes to read the selection may show up.

Limitations:

- Inside windows of programs running as administrator, CapsLock works as plain CapsLock unless 202020 also runs as administrator.
- If CapsLock is released while such a window or the lock screen has focus, 202020 misses the release and keeps the layer on; tap CapsLock to clear it.

## Running

`202020.exe` is portable: `config.toml` (and logs, if enabled) live next to the exe, so put it in a writable folder. Keep `capsule_hook.dll` next to it: while the capsule bar runs or the launcher's index is loaded, it is loaded into explorer to pass tray icons to the bar and to do the shell work (taskbar pins, program icons, the All apps list, opening items) there, so 202020 does not load the shell itself. To start at login, put a shortcut to it in `shell:startup`.

| Flag | Effect |
|---|---|
| `--safe` | Don't hide the native taskbar (useful for a first test of the capsule bar) |
| `--restore` | Restore the native taskbar (after both 202020 and its watchdog were killed) |

## Config

`config.toml` next to the exe is created on first run. Capsule bar settings live under `[capsule_bar]`, launcher settings under `[launcher]`, CapsLock hyper settings under `[hyper]`. While the bar is running, changes apply on save; `enabled` is read at startup and set by the tray menu. See the comments in the file. If acrylic stops working (e.g. the capsule stays a solid color), set `backdrop` to `"accent"`.

To report a problem, set `level = "debug"` under `[logging]` and restart 202020. Logs go to `logs\log.txt` (and `logs\watchdog.txt` for the watchdog), including panics and crashes. `"trace"` also logs why each window was shown on or left off the capsule bar.

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
