//! Default render endpoint volume. Core Audio calls back on its own MTA
//! threads; callbacks only store values and post to the UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{LPARAM, PROPERTYKEY, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{GUID, PCWSTR, Result, implement};

use crate::util::hwnd;

struct State {
    level: AtomicU32,
    muted: AtomicBool,
    notify: AtomicIsize,
    volume_msg: u32,
    device_msg: u32,
}

impl State {
    fn post(&self, msg: u32) {
        unsafe {
            let _ = PostMessageW(Some(hwnd(self.notify.load(Ordering::Relaxed))), msg, WPARAM(0), LPARAM(0));
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback(Arc<State>);

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            self.0.level.store(d.fMasterVolume.to_bits(), Ordering::Relaxed);
            self.0.muted.store(d.bMuted.as_bool(), Ordering::Relaxed);
            self.0.post(self.0.volume_msg);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback(Arc<State>);

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            self.0.post(self.0.device_msg);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

pub struct Volume {
    state: Arc<State>,
    enumerator: Option<IMMDeviceEnumerator>,
    device_cb: Option<IMMNotificationClient>,
    endpoint: Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
}

impl Volume {
    pub fn new(notify: isize, volume_msg: u32, device_msg: u32) -> Self {
        let state = Arc::new(State {
            level: AtomicU32::new(0),
            muted: AtomicBool::new(true),
            notify: AtomicIsize::new(notify),
            volume_msg,
            device_msg,
        });
        let enumerator: Option<IMMDeviceEnumerator> =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let device_cb: Option<IMMNotificationClient> = enumerator.as_ref().and_then(|e| {
            let cb: IMMNotificationClient = DeviceCallback(state.clone()).into();
            unsafe { e.RegisterEndpointNotificationCallback(&cb) }.ok().map(|_| cb)
        });
        let mut v = Volume { state, enumerator, device_cb, endpoint: None };
        v.attach();
        v
    }

    pub fn attach(&mut self) {
        self.detach();
        let Some(e) = &self.enumerator else { return };
        let bound = unsafe {
            e.GetDefaultAudioEndpoint(eRender, eConsole)
                .and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None))
        };
        match bound {
            Ok(ep) => {
                let cb: IAudioEndpointVolumeCallback = VolumeCallback(self.state.clone()).into();
                unsafe {
                    let _ = ep.RegisterControlChangeNotify(&cb);
                    let level = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
                    let muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                    self.state.level.store(level.to_bits(), Ordering::Relaxed);
                    self.state.muted.store(muted, Ordering::Relaxed);
                }
                self.endpoint = Some((ep, cb));
            }
            Err(e) => {
                log::info!("no audio endpoint: {e}");
                self.state.muted.store(true, Ordering::Relaxed);
                self.state.level.store(0, Ordering::Relaxed);
            }
        }
    }

    fn detach(&mut self) {
        if let Some((ep, cb)) = self.endpoint.take() {
            unsafe {
                let _ = ep.UnregisterControlChangeNotify(&cb);
            }
        }
    }

    pub fn available(&self) -> bool {
        self.endpoint.is_some()
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.state.level.load(Ordering::Relaxed))
    }

    pub fn muted(&self) -> bool {
        self.state.muted.load(Ordering::Relaxed)
    }

    pub fn change(&self, delta: f32) {
        if let Some((ep, _)) = &self.endpoint {
            unsafe {
                let level = (ep.GetMasterVolumeLevelScalar().unwrap_or(self.level()) + delta).clamp(0.0, 1.0);
                if self.muted() && delta > 0.0 {
                    let _ = ep.SetMute(false, &GUID::zeroed());
                }
                let _ = ep.SetMasterVolumeLevelScalar(level, &GUID::zeroed());
            }
        }
    }
}

impl Drop for Volume {
    fn drop(&mut self) {
        self.detach();
        if let (Some(e), Some(cb)) = (&self.enumerator, &self.device_cb) {
            unsafe {
                let _ = e.UnregisterEndpointNotificationCallback(cb);
            }
        }
    }
}
