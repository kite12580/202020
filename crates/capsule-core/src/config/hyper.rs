use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

use super::defaults::{DEFAULT_ALT_KEYS, DEFAULT_KEYS, DEFAULT_WIN_KEYS};
use crate::hyper::{self, Action, Binding, Job, Keymap};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hyper {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "slow_mouse_speed")]
    pub slow_mouse_speed: u32,
    #[serde(default = "yes")]
    pub block_ctrl_wheel: bool,
    #[serde(default = "terminals")]
    pub terminals: Vec<String>,
    #[serde(default)]
    pub hotstrings: BTreeMap<String, String>,
    #[serde(default = "keys")]
    pub keys: Keymap,
    #[serde(default = "alt_keys")]
    pub alt_keys: Keymap,
    #[serde(default = "win_keys", deserialize_with = "parse_win_keys")]
    pub win_keys: Keymap,
}

fn yes() -> bool {
    true
}

fn slow_mouse_speed() -> u32 {
    1
}

fn terminals() -> Vec<String> {
    vec!["WindowsTerminal.exe".into()]
}

fn keys() -> Keymap {
    toml::from_str(DEFAULT_KEYS).expect("default keys")
}

fn alt_keys() -> Keymap {
    toml::from_str(DEFAULT_ALT_KEYS).expect("default alt_keys")
}

fn win_keys() -> Keymap {
    let table = toml::from_str(DEFAULT_WIN_KEYS).expect("default win_keys");
    win_keymap(table, |e| log::warn!("hyper win_keys: {e}"))
}

impl Default for Hyper {
    fn default() -> Self {
        Self {
            enabled: yes(),
            slow_mouse_speed: slow_mouse_speed(),
            block_ctrl_wheel: yes(),
            terminals: terminals(),
            hotstrings: BTreeMap::new(),
            keys: keys(),
            alt_keys: alt_keys(),
            win_keys: win_keys(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingTable {
    #[serde(rename = "do")]
    action: Option<String>,
    wrap: Option<[String; 2]>,
    #[serde(default)]
    at_end: bool,
    text: Option<String>,
    wheel: Option<i32>,
    terminal: Option<String>,
}

fn action(s: &str) -> Result<Action, String> {
    hyper::action(s).ok_or_else(|| format!("{s:?} is neither keys nor an action"))
}

fn binding(v: toml::Value) -> Result<Binding, String> {
    if let toml::Value::String(s) = &v {
        return Ok(Binding { action: action(s)?, terminal: None });
    }
    let t: BindingTable = v.try_into().map_err(|e: toml::de::Error| e.message().to_owned())?;
    let actions = [
        t.action.as_deref().map(action).transpose()?,
        t.wrap.map(|[open, close]| Action::Job(Job::Wrap { open, close, at_end: t.at_end })),
        t.text.map(|text| Action::Job(Job::Paste(text))),
        t.wheel.map(Action::Wheel),
    ];
    let mut actions = actions.into_iter().flatten();
    let (Some(main), None) = (actions.next(), actions.next()) else {
        return Err("needs exactly one of do, wrap, text and wheel".into());
    };
    Ok(Binding { action: main, terminal: t.terminal.as_deref().map(action).transpose()? })
}

fn keymap(table: BTreeMap<String, toml::Value>, mut skipped: impl FnMut(String)) -> Keymap {
    table
        .into_iter()
        .filter_map(|(name, v)| {
            let Some(key) = crate::launcher::hotkey::key(&name) else {
                skipped(format!("{name:?}: unknown key"));
                return None;
            };
            binding(v).map(|b| (key as u16, b)).map_err(|e| skipped(format!("{name:?}: {e}"))).ok()
        })
        .collect()
}

fn win_keymap(table: BTreeMap<String, toml::Value>, mut skipped: impl FnMut(String)) -> Keymap {
    table
        .into_iter()
        .filter_map(|(name, v)| match (crate::launcher::hotkey::key(&name), v.as_str().and_then(hyper::action)) {
            (Some(key), Some(action @ Action::Keys(_))) => Some((key as u16, Binding { action, terminal: None })),
            (None, _) => {
                skipped(format!("{name:?}: unknown key"));
                None
            }
            _ => {
                skipped(format!("{name:?}: needs keys to press"));
                None
            }
        })
        .collect()
}

fn parse_win_keys<'de, D: Deserializer<'de>>(d: D) -> Result<Keymap, D::Error> {
    Ok(win_keymap(BTreeMap::deserialize(d)?, |e| log::warn!("hyper win_keys: {e}")))
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(keymap(BTreeMap::deserialize(d)?, |e| log::warn!("hyper keys: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    #[test]
    fn keymaps() {
        #[derive(Deserialize)]
        struct Doc {
            hyper: BTreeMap<String, BTreeMap<String, toml::Value>>,
        }
        for text in [DEFAULT_KEYS, DEFAULT_ALT_KEYS] {
            let table: BTreeMap<String, toml::Value> = toml::from_str(text).unwrap();
            let n = table.len();
            assert_eq!(keymap(table, |e| panic!("{e}")).len(), n);
        }
        let table: BTreeMap<String, toml::Value> = toml::from_str(DEFAULT_WIN_KEYS).unwrap();
        let n = table.len();
        assert_eq!(win_keymap(table, |e| panic!("{e}")).len(), n);
        let mut skipped = Vec::new();
        let table = toml::from_str("x = 'Win+1'\nbogus = 'End'\ny = 'reload'\nz = { do = 'End' }").unwrap();
        assert_eq!(win_keymap(table, |e| skipped.push(e)).len(), 1);
        assert_eq!(skipped.len(), 3, "{skipped:?}");
        let h = parse("[hyper]\nblock_ctrl_wheel = false\n[hyper.win_keys]").unwrap().hyper;
        assert!(!h.block_ctrl_wheel && h.win_keys.is_empty());
        assert_eq!(parse("").unwrap().hyper.win_keys, Hyper::default().win_keys);

        let text = "[hyper.keys]\nh = 'Home'\nbogus = 'End'\nj = { do = 'End', wheel = 1 }\nk = { wrap = ['a'] }\n\
                    l = { do = 'Left', terminal = 'nope' }\nm = { tetx = 'x' }\nn = 'Ctrl+Bogus'";
        let table: Doc = toml::from_str(text).unwrap();
        let mut skipped = Vec::new();
        let keys = keymap(table.hyper["keys"].clone(), |e| skipped.push(e));
        assert_eq!(keys.len(), 1);
        assert_eq!(skipped.len(), 6, "{skipped:?}");
        let h = parse(text).unwrap().hyper;
        assert_eq!(h.keys, keys);
        assert_eq!(h.alt_keys, Hyper::default().alt_keys);
    }
}
