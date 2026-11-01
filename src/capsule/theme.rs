use std::ffi::c_void;

use capsule_core::config::Backdrop;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWINDOWATTRIBUTE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, PCWSTR, s, w};

use crate::gfx::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub dark: bool,
    pub transparency: bool,
    pub high_contrast: bool,
}

pub struct Palette {
    pub tint: Color,
    pub text: Color,
    pub hover: Color,
    pub active: Color,
}

fn personalize_dword(name: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = 4u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut c_void),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(data)
}

impl Theme {
    pub fn read() -> Theme {
        let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some(&mut hc as *mut _ as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
        Theme {
            dark: personalize_dword(w!("SystemUsesLightTheme")) != Some(1),
            transparency: personalize_dword(w!("EnableTransparency")) != Some(0),
            high_contrast: hc.dwFlags.contains(HCF_HIGHCONTRASTON),
        }
    }

    fn solid(&self) -> bool {
        self.high_contrast || !self.transparency
    }

    pub fn palette(&self, opacity: f32) -> Palette {
        let opacity = if self.solid() { 1.0 } else { opacity };
        if self.dark {
            Palette {
                tint: Color(0.125, 0.125, 0.125, opacity),
                text: Color(1.0, 1.0, 1.0, 0.92),
                hover: Color(1.0, 1.0, 1.0, 0.08),
                active: Color(1.0, 1.0, 1.0, 0.15),
            }
        } else {
            Palette {
                tint: Color(0.953, 0.953, 0.953, opacity),
                text: Color(0.0, 0.0, 0.0, 0.9),
                hover: Color(0.0, 0.0, 0.0, 0.05),
                active: Color(0.0, 0.0, 0.0, 0.1),
            }
        }
    }
}

fn set_attr<T>(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: &T) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, attr, value as *const T as *const c_void, size_of::<T>() as u32);
    }
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

fn set_accent(hwnd: HWND, state: u32, gradient: u32) {
    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let f: SetWca = std::mem::transmute(f);
        let mut policy = AccentPolicy { state, flags: 2, gradient, animation: 0 };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut _ as *mut c_void,
            size: size_of::<AccentPolicy>(),
        };
        let _ = f(hwnd, &mut data);
    }
}

/// Applies backdrop, corner, and dark-mode attributes. Content pixels with
/// zero alpha show the backdrop.
pub fn apply(hwnd: HWND, backdrop: Backdrop, theme: &Theme) {
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(
            hwnd,
            &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 },
        );
    }
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(theme.dark));
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND);
    set_attr(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
    let backdrop = if theme.solid() { Backdrop::None } else { backdrop };
    match backdrop {
        Backdrop::Acrylic => {
            set_accent(hwnd, ACCENT_DISABLED, 0);
            set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_TRANSIENTWINDOW);
        }
        Backdrop::Accent => {
            set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE);
            let tint = if theme.dark { 0x0120_2020 } else { 0x01F3_F3F3 };
            set_accent(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND, tint);
        }
        Backdrop::None => {
            set_accent(hwnd, ACCENT_DISABLED, 0);
            set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE);
        }
    }
}
