macro_rules! default_keys {
    (keys) => {
        r#"# CapsLock + key. Without this table these defaults apply; with it, keys left out of it do nothing.
# A value is either keys to press, separated by spaces ("End Enter", "Ctrl+Shift+Left"), or one of the actions
# copy_or_line, paste_plain, delete_line, hotstring, escape_or_move, power_options, topmost, caption, reload,
# transparency, switch_desktop, virgo_desktop, left_button, middle_button and right_button (held while the key is).
# A table instead holds one of:
#   do = keys or an action
#   wrap = [open, close]   wraps the selection, or inserts both; with at_end = true, at the end of the line, then Enter
#   text = "..."           types the text
#   wheel = n              turns the wheel n notches, negative is down
# plus terminal = ... for the programs in terminals.
h = "Left"
j = "Down"
k = "Up"
l = "Right"
e = "Ctrl+Right"
b = "Ctrl+Left"
";" = "Home"
"'" = "End"
enter = "End"
space = "Right"
g = "Ctrl+End"
r = "PageUp"
q = "PageDown"
n = "Ctrl+Shift+Tab"
m = "Ctrl+Tab"
o = "End Enter"
u = { do = "Ctrl+Z", terminal = "Ctrl+/" }
z = "Ctrl+S"
c = "Alt+Tab"
v = "Ctrl+Shift+V"
"/" = "Ctrl+F"
1 = "left_button"
2 = "middle_button"
3 = "right_button"
"," = "BrowserBack"
"." = "BrowserForward"
"`" = "Apps"
w = { do = "Ctrl+Shift+Left Backspace", terminal = "Ctrl+W" }
backspace = "Ctrl+Left Ctrl+Shift+Right Backspace"
d = { do = "delete_line", terminal = "Ctrl+U" }
x = "Ctrl+Insert Delete"
y = "copy_or_line"
p = "Shift+Insert"
9 = { wrap = ["(", ")"] }
0 = { text = ")" }
"[" = { wrap = [" {", "}"], at_end = true }
"=" = { wrap = [' + "', '" + '] }
tab = "hotstring"
f = "escape_or_move"
f1 = "topmost"
f3 = "MediaNext"
f4 = "transparency"
f5 = "reload"
f9 = "power_options"
"#
    };
    (alt_keys) => {
        r#"# CapsLock + left Alt + key, written as above.
h = "Shift+Left"
j = "Shift+Down"
k = "Shift+Up"
l = "Shift+Right"
e = "Ctrl+Shift+Right"
b = "Ctrl+Shift+Left"
";" = "Shift+Home"
"'" = "Shift+End"
g = "Ctrl+Home"
r = { wheel = 2 }
q = { wheel = -2 }
o = "Home Enter Up"
u = "Ctrl+Y"
z = "Ctrl+Shift+S"
v = "switch_desktop"
w = "End Shift+Down Shift+Home Backspace"
d = "End Shift+Home Delete"
x = "Ctrl+Insert Backspace"
y = "copy_or_line"
p = "paste_plain"
9 = { wrap = ['"', '"'] }
0 = { wrap = ["'", "'"] }
"[" = { wrap = ["[", "]"] }
f1 = "caption"
f2 = "MediaPlayPause"
f3 = "MediaPrev"
"#
    };
    (win_keys) => {
        r#"# Win + key with no other modifier, mapped to keys to press instead, or to virgo_desktop: tap it to switch
# between two desktops, hold it to send the active window to the other one. Without this table these defaults apply;
# leave it empty to keep Windows' own shortcuts.
e = "Win+2"
t = "Ctrl+Shift+T"
"`" = "virgo_desktop"
"#
    };
}

pub(super) const DEFAULT_KEYS: &str = default_keys!(keys);
pub(super) const DEFAULT_ALT_KEYS: &str = default_keys!(alt_keys);
pub(super) const DEFAULT_WIN_KEYS: &str = default_keys!(win_keys);

pub const DEFAULT_TEXT: &str = concat!(
    r#"# 202020 settings. Capsule bar changes apply on save while it is running.

[capsule_bar]
# Replace the Windows taskbar with the capsule bar. Toggle it from the tray menu.
enabled = false

[capsule_bar.position]
# Horizontal center of the capsule as a fraction of the screen width (0.0 – 1.0). Updated when dragged.
# 0.0 and 1.0 keep the capsule against the left or right edge as it grows and shrinks.
center_x = 0.5
# Distance from the capsule to the bottom of the screen (DIPs). Updated when dragged.
bottom_gap = 0.0

[capsule_bar.appearance]
# "acrylic" (system acrylic), "accent" (legacy blur, for when acrylic doesn't work) or "none" (plain transparent).
backdrop = "none"
# Opacity of the background tint (0.0 – 1.0).
opacity = 0.8

[capsule_bar.volume]
# Volume change per wheel notch, in percent.
step = 2

[capsule_bar.keyboard.labels]
# Override keyboard layout labels by ISO 639 language code. Built in: zh = "中", ja = "あ", ko = "한"; others show the
# uppercase code. An IME in English mode always shows "英" (Chinese) or "A" (Japanese, Korean).
# en = "EN"

[capsule_bar.tray]
# Tray icons shown on the capsule. Right-click an icon in the ^ popup to pin or unpin it.
pinned = ["202020.exe|1"]

[launcher]
# Spotlight-style search, opened with the hotkey. Toggle it from the tray menu.
enabled = true
# Modifiers (Alt, Ctrl, Shift, Win) and a key, e.g. "Alt+Space" or "Ctrl+Shift+K". Applied by Reload config.
hotkey = "Alt+Space"
# Rows shown at most (1 – 20).
max_results = 8
# Free the search index after this many minutes without opening the launcher.
idle_unload_minutes = 10

[launcher.prefixes]
# Typing a prefix first searches only that source. "" turns a prefix off.
apps = ""
commands = ">"
folders = "/"
windows = "@"

# Each source can be turned off; its weight scales its results' ranking (0.0 – 2.0).
[launcher.apps]
enabled = false
weight = 1.0

[launcher.windows]
enabled = false
weight = 0.9

[launcher.commands]
enabled = true
weight = 0.8
custom = []
# Your own entries:
# [[launcher.commands.custom]]
# name = "Calculator 计算器"
# command = "calc.exe"
# args = ""

[launcher.folders]
enabled = true
weight = 1.0

# Folders whose files and subfolders are searchable by name. Repeat the table for more roots.
[[launcher.folders.roots]]
# %VARIABLES% are expanded.
path = "C:\\Games"
# Subfolder levels to descend; 0 searches only the folder itself.
depth = 1
# Only files with these extensions, e.g. ["pdf", "docx"]; [] means all.
extensions = ["exe", "lnk"]
# File or folder names to skip; * and ? are wildcards.
exclude = []
# Also list folders themselves.
include_dirs = false
# Stop scanning this root after this many entries.
max_items = 20000

[hyper]
# Hold CapsLock for the hyper layer, tap it for Esc, press Shift+CapsLock for CapsLock. Toggle it from the tray menu. Applied by Reload config.
enabled = true
# Mouse speed (1 – 20) while CapsLock and left Alt are held.
slow_mouse_speed = 1
# Ctrl + mouse wheel does nothing instead of zooming.
block_ctrl_wheel = true
# Programs that get the terminal = ... variant of a key below.
terminals = ["WindowsTerminal.exe"]

[hyper.hotstrings]
# Type a key, then CapsLock+Tab replaces it with the text. Keys match the end of the line, ignoring case.
# sig = "Best regards,\nMe"

"#,
    "[hyper.keys]\n",
    default_keys!(keys),
    "\n[hyper.alt_keys]\n",
    default_keys!(alt_keys),
    "\n[hyper.win_keys]\n",
    default_keys!(win_keys),
    r#"
[logging]
# Write logs to the logs folder next to the exe: "off", "error", "warn", "info", "debug" or "trace". Use "debug" when
# reporting a problem; "trace" also logs why each window was shown on or left off the capsule bar. Read at startup.
level = "off"
"#
);
