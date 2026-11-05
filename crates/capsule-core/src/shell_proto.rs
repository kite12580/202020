//! Shell work the bar and launcher ask `capsule_hook.dll` to do inside
//! explorer, and the answers, as `WM_COPYDATA` payloads. Requests go to
//! `Shell_TrayWnd` (wParam: the requesting window), answers to the relay
//! window (wParam: the window that made the request).

/// Distinct from the `dwData` values `Shell_TrayWnd` itself handles.
pub const COPYDATA_REQUEST: usize = 0x2020_0001;
pub const COPYDATA_ANSWER: usize = 0x2020_0002;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Pins,
    /// The Start menu's All apps list.
    Apps,
    /// `name` is a path or a `shell:AppsFolder\…` name.
    Icon {
        id: u64,
        px: u32,
        name: String,
    },
    /// `ShellExecuteEx`, without an answer.
    Open {
        file: String,
        params: String,
        dir: String,
    },
}

/// A program pinned to the native taskbar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PinInfo {
    pub aumid: Option<String>,
    pub target: Option<String>,
    pub parsing_name: String,
    pub name: String,
}

/// An entry of the All apps list; `id` is its parsing name in the Apps folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub id: String,
}

/// Top-down BGRA pixels, with the source bitmap's bits per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bpp: u16,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Pins(Vec<PinInfo>),
    Apps(Vec<AppInfo>),
    Icon { id: u64, bitmap: Option<Bitmap> },
}

const PINS: u32 = 0;
const ICON: u32 = 1;
const APPS: u32 = 2;
const OPEN: u32 = 3;
const NONE: u32 = u32::MAX;

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn str(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u32(units.len() as u32);
        units.iter().for_each(|u| self.0.extend_from_slice(&u.to_le_bytes()));
    }

    fn opt_str(&mut self, s: Option<&str>) {
        match s {
            Some(s) => self.str(s),
            None => self.u32(NONE),
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.bytes(8)?.try_into().ok()?))
    }

    fn units(&mut self, n: u32) -> Option<String> {
        let units: Vec<u16> =
            self.bytes(n as usize * 2)?.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        Some(String::from_utf16_lossy(&units))
    }

    fn str(&mut self) -> Option<String> {
        let n = self.u32()?;
        self.units(n)
    }

    fn opt_str(&mut self) -> Option<Option<String>> {
        match self.u32()? {
            NONE => Some(None),
            n => self.units(n).map(Some),
        }
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Request::Pins => w.u32(PINS),
            Request::Apps => w.u32(APPS),
            Request::Open { file, params, dir } => {
                w.u32(OPEN);
                w.str(file);
                w.str(params);
                w.str(dir);
            }
            Request::Icon { id, px, name } => {
                w.u32(ICON);
                w.u64(*id);
                w.u32(*px);
                w.str(name);
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<Request> {
        let mut r = Reader(bytes);
        match r.u32()? {
            PINS => Some(Request::Pins),
            APPS => Some(Request::Apps),
            OPEN => Some(Request::Open { file: r.str()?, params: r.str()?, dir: r.str()? }),
            ICON => Some(Request::Icon { id: r.u64()?, px: r.u32()?, name: r.str()? }),
            _ => None,
        }
    }
}

impl Answer {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Answer::Pins(pins) => {
                w.u32(PINS);
                w.u32(pins.len() as u32);
                for p in pins {
                    w.opt_str(p.aumid.as_deref());
                    w.opt_str(p.target.as_deref());
                    w.str(&p.parsing_name);
                    w.str(&p.name);
                }
            }
            Answer::Apps(apps) => {
                w.u32(APPS);
                w.u32(apps.len() as u32);
                for a in apps {
                    w.str(&a.name);
                    w.str(&a.id);
                }
            }
            Answer::Icon { id, bitmap } => {
                w.u32(ICON);
                w.u64(*id);
                match bitmap {
                    Some(b) => {
                        w.u32(b.width);
                        w.u32(b.height);
                        w.u32(u32::from(b.bpp));
                        w.0.extend_from_slice(&b.pixels);
                    }
                    None => w.u32(0),
                }
            }
        }
        w.0
    }

    pub fn decode(bytes: &[u8]) -> Option<Answer> {
        let mut r = Reader(bytes);
        match r.u32()? {
            PINS => {
                let n = r.u32()?;
                let mut pins = Vec::new();
                for _ in 0..n {
                    pins.push(PinInfo {
                        aumid: r.opt_str()?,
                        target: r.opt_str()?,
                        parsing_name: r.str()?,
                        name: r.str()?,
                    });
                }
                Some(Answer::Pins(pins))
            }
            APPS => {
                let n = r.u32()?;
                let mut apps = Vec::new();
                for _ in 0..n {
                    apps.push(AppInfo { name: r.str()?, id: r.str()? });
                }
                Some(Answer::Apps(apps))
            }
            ICON => {
                let id = r.u64()?;
                let width = r.u32()?;
                if width == 0 {
                    return Some(Answer::Icon { id, bitmap: None });
                }
                let height = r.u32()?;
                let bpp = u16::try_from(r.u32()?).ok()?;
                let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
                let pixels = r.bytes(len)?.to_vec();
                Some(Answer::Icon { id, bitmap: Some(Bitmap { width, height, bpp, pixels }) })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let open = Request::Open { file: r"C:\a b\c.exe".into(), params: String::new(), dir: r"C:\a b".into() };
        for req in
            [Request::Pins, Request::Apps, Request::Icon { id: 7, px: 30, name: r"shell:AppsFolder\微信".into() }, open]
        {
            assert_eq!(Request::decode(&req.encode()), Some(req));
        }
    }

    #[test]
    fn answers_round_trip() {
        let pins = Answer::Pins(vec![
            PinInfo { aumid: Some("App.Id".into()), target: None, parsing_name: "x".into(), name: "App".into() },
            PinInfo { aumid: None, target: Some(String::new()), parsing_name: String::new(), name: "y".into() },
        ]);
        let icon =
            Answer::Icon { id: 3, bitmap: Some(Bitmap { width: 2, height: 1, bpp: 32, pixels: (0..8).collect() }) };
        let apps = Answer::Apps(vec![AppInfo { name: "微信".into(), id: "Tencent.WeChat".into() }, AppInfo::default()]);
        for a in [pins, apps, icon, Answer::Icon { id: 4, bitmap: None }] {
            assert_eq!(Answer::decode(&a.encode()), Some(a));
        }
    }

    #[test]
    fn truncated_or_unknown() {
        let icon = Answer::Icon { id: 3, bitmap: Some(Bitmap { width: 2, height: 2, bpp: 32, pixels: vec![0; 16] }) };
        let bytes = icon.encode();
        assert_eq!(Answer::decode(&bytes[..bytes.len() - 1]), None);
        let req = Request::Icon { id: 1, px: 16, name: "abc".into() }.encode();
        assert_eq!(Request::decode(&req[..req.len() - 2]), None);
        assert_eq!(Request::decode(&9u32.to_le_bytes()), None);
        assert_eq!(Answer::decode(&[]), None);
    }
}
