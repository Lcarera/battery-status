//! Tray: hidden window, message loop, one tray icon per device, blink, and menu.

use crate::alerts::AlertLevel;
use crate::app::{Action, App};
use crate::device::{self, DeviceEvent, DeviceId, DeviceKind};
use crate::icon::{self, BarColor};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, PCWSTR, w};

pub const WM_APP_EVENT: u32 = WM_APP + 1;
const WM_TRAY_CALLBACK: u32 = WM_APP + 2;
const BLINK_TIMER: usize = 1;
const BLINK_MS: u32 = 500;
const TICK_TIMER: usize = 2;
const TICK_MS: u32 = 5000;
const ID_AUTOSTART: usize = 1;
const ID_SETTINGS: usize = 2;
const ID_EXIT: usize = 3;
const ID_DEVICE_BASE: usize = 100;
/// NIN_SELECT | NINF_KEY (not exported by the windows crate).
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;

/// Messages other threads send to the tray thread (then call `post_wakeup`).
pub enum TrayMsg {
    Device(DeviceEvent),
    Acknowledge(DeviceId),
    ReloadConfig(String),
}

/// Side effects the tray asks for. `main` sets them (Task 11).
pub struct Hooks {
    pub show_toast: Box<dyn Fn(DeviceId, &str, u8, AlertLevel)>,
    pub on_resume: Box<dyn Fn()>,
    pub autostart_enabled: Box<dyn Fn() -> bool>,
    pub set_autostart: Box<dyn Fn(bool)>,
    pub open_settings: Box<dyn Fn()>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            show_toast: Box::new(|_, name, p, l| log::info!("toast: {name} {p}% {l:?}")),
            on_resume: Box::new(|| log::info!("resume")),
            autostart_enabled: Box::new(|| false),
            set_autostart: Box::new(|on| log::info!("set autostart {on}")),
            open_settings: Box::new(|| log::info!("open settings")),
        }
    }
}

fn fill_wide(buf: &mut [u16], text: &str) {
    let max = buf.len().saturating_sub(1);
    let mut n = 0;
    for u in text.encode_utf16().take(max) {
        buf[n] = u;
        n += 1;
    }
    if !buf.is_empty() {
        buf[n] = 0;
    }
}

fn to_guid(v: u128) -> GUID {
    GUID::from_u128(v)
}

struct Slot {
    uid: u32,
    guid: GUID,
    kind: DeviceKind,
    percent: u8,
    color: BarColor,
    tooltip: String,
    hicon: HICON,
}

struct State {
    app: App,
    rx: Receiver<TrayMsg>,
    hwnd: HWND,
    hooks: Hooks,
    exe_path: String,
    taskbar_created: u32,
    slots: HashMap<DeviceId, Slot>,
    next_uid: u32,
    blink_dim: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs `f` on the state. Returns `None` when the state is busy (re-entrant call).
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn make_icon(kind: DeviceKind, percent: u8, color: BarColor, dimmed: bool) -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let px = icon::render(kind, percent, color, size, dimmed);
    let mask = vec![0u8; (size * size / 8) as usize + 16];
    unsafe {
        let color_bm = CreateBitmap(size as i32, size as i32, 1, 32, Some(px.as_ptr().cast()));
        let mask_bm = CreateBitmap(size as i32, size as i32, 1, 1, Some(mask.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask_bm, hbmColor: color_bm };
        let h = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(color_bm.0));
        let _ = DeleteObject(HGDIOBJ(mask_bm.0));
        h
    }
}

impl State {
    fn nid(&self, slot: &Slot) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: slot.uid,
            uFlags: NIF_GUID | NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY_CALLBACK,
            hIcon: slot.hicon,
            guidItem: slot.guid,
            ..Default::default()
        };
        fill_wide(&mut nid.szTip, &slot.tooltip);
        nid
    }

    fn shell(&self, msg: NOTIFY_ICON_MESSAGE, slot: &Slot) -> bool {
        let nid = self.nid(slot);
        unsafe { Shell_NotifyIconW(msg, &nid).as_bool() }
    }

    fn add(&self, slot: &Slot) {
        if !self.shell(NIM_ADD, slot) {
            // A stale icon with this GUID can remain after a crash: delete it and try once more.
            self.shell(NIM_DELETE, slot);
            if !self.shell(NIM_ADD, slot) {
                log::error!("tray icon add failed: {}", slot.tooltip);
                return;
            }
        }
        log::info!("tray icon added: {}", slot.tooltip);
        let mut nid = self.nid(slot);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn show_icon(&mut self, id: DeviceId, kind: DeviceKind, percent: u8, color: BarColor, tooltip: String) {
        let dimmed = self.blink_dim && self.app.is_blinking(id);
        let hicon = make_icon(kind, percent, color, dimmed);
        if let Some(mut slot) = self.slots.remove(&id) {
            let old = slot.hicon;
            slot.kind = kind;
            slot.percent = percent;
            slot.color = color;
            slot.tooltip = tooltip;
            slot.hicon = hicon;
            self.shell(NIM_MODIFY, &slot);
            self.slots.insert(id, slot);
            unsafe {
                let _ = DestroyIcon(old);
            }
        } else {
            self.next_uid += 1;
            let guid = to_guid(device::icon_guid(id, &self.exe_path));
            let slot = Slot { uid: self.next_uid, guid, kind, percent, color, tooltip, hicon };
            self.add(&slot);
            self.slots.insert(id, slot);
        }
    }

    fn redraw(&mut self, id: DeviceId) {
        if let Some(s) = self.slots.get(&id) {
            let (k, p, c, t) = (s.kind, s.percent, s.color, s.tooltip.clone());
            self.show_icon(id, k, p, c, t);
        }
    }

    fn remove_icon(&mut self, id: DeviceId) {
        if let Some(slot) = self.slots.remove(&id) {
            log::info!("tray icon removed: {}", slot.tooltip);
            self.shell(NIM_DELETE, &slot);
            unsafe {
                let _ = DestroyIcon(slot.hicon);
            }
        }
    }

    fn apply(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::ShowIcon { id, kind, percent, color, tooltip } => self.show_icon(id, kind, percent, color, tooltip),
                Action::RemoveIcon(id) => self.remove_icon(id),
                Action::StartBlink(_) => unsafe {
                    SetTimer(Some(self.hwnd), BLINK_TIMER, BLINK_MS, None);
                },
                Action::StopBlink(id) => self.redraw(id),
                Action::ShowToast { id, name, percent, level } => (self.hooks.show_toast)(id, &name, percent, level),
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            let actions = match msg {
                TrayMsg::Device(ev) => self.app.handle(ev),
                TrayMsg::Acknowledge(id) => self.app.acknowledge(id),
                TrayMsg::ReloadConfig(text) => match self.app.reload_config(&text) {
                    Ok(a) => a,
                    Err(e) => {
                        log::error!("settings file not applied: {e}");
                        Vec::new()
                    }
                },
            };
            self.apply(actions);
        }
    }

    fn blink_tick(&mut self) {
        self.blink_dim = !self.blink_dim;
        let ids: Vec<DeviceId> = self.slots.keys().copied().filter(|&id| self.app.is_blinking(id)).collect();
        if ids.is_empty() {
            self.blink_dim = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), BLINK_TIMER);
            }
        }
        for id in ids {
            self.redraw(id);
        }
    }

    fn readd_all(&self) {
        for slot in self.slots.values() {
            self.add(slot);
        }
    }

    fn device_for_uid(&self, uid: u32) -> Option<DeviceId> {
        self.slots.iter().find(|(_, s)| s.uid == uid).map(|(&id, _)| id)
    }

    fn exit(&mut self) {
        let ids: Vec<DeviceId> = self.slots.keys().copied().collect();
        for id in ids {
            self.remove_icon(id);
        }
        unsafe { PostQuitMessage(0) };
    }
}

fn show_menu(hwnd: HWND) {
    let Some((items, autostart)) = with_state(|s| (s.app.menu_items(), (s.hooks.autostart_enabled)())) else {
        return;
    };
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for (i, text) in items.iter().enumerate() {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, ID_DEVICE_BASE + i, PCWSTR(wide.as_ptr()));
        }
        if items.is_empty() {
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, ID_DEVICE_BASE, w!("No devices"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let check = if autostart { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | check, ID_AUTOSTART, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Open settings file"));
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    };
    with_state(|s| match cmd {
        ID_AUTOSTART => (s.hooks.set_autostart)(!autostart),
        ID_SETTINGS => (s.hooks.open_settings)(),
        ID_EXIT => s.exit(),
        _ => {}
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let repost = || unsafe {
        let _ = PostMessageW(Some(hwnd), msg, wparam, lparam);
    };
    match msg {
        WM_APP_EVENT => {
            if with_state(|s| s.drain()).is_none() {
                repost();
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == BLINK_TIMER => {
            with_state(|s| s.blink_tick());
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TICK_TIMER => {
            with_state(|s| {
                let a = s.app.tick(std::time::Instant::now());
                s.apply(a);
            });
            LRESULT(0)
        }
        WM_TRAY_CALLBACK => {
            let event = (lparam.0 as u32) & 0xFFFF;
            let uid = ((lparam.0 as u32) >> 16) & 0xFFFF;
            match event {
                NIN_SELECT | NIN_KEYSELECT => {
                    with_state(|s| {
                        if let Some(id) = s.device_for_uid(uid) {
                            let a = s.app.acknowledge(id);
                            s.apply(a);
                        }
                    });
                }
                WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        // The installer and the uninstaller send WM_CLOSE to stop a running copy.
        // Without this, DefWindowProc destroys the window and the process stays.
        WM_CLOSE => {
            if with_state(|s| s.exit()).is_none() {
                repost();
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                with_state(|s| (s.hooks.on_resume)());
            }
            LRESULT(1)
        }
        _ => {
            let taskbar_created = with_state(|s| s.taskbar_created).unwrap_or(0);
            if taskbar_created != 0 && msg == taskbar_created {
                with_state(|s| s.readd_all());
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Wakes the tray thread after a `TrayMsg` was sent. Safe to call from any thread.
pub fn post_wakeup(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_EVENT, WPARAM(0), LPARAM(0));
    }
}

pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    /// Creates the hidden window. Call on the thread that will call `run`.
    pub fn new(app: App, rx: Receiver<TrayMsg>, hooks: Hooks) -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("BatteryStatusTrayWindow");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            // A normal (hidden) top-level window: message-only windows do not get
            // the TaskbarCreated and WM_POWERBROADCAST broadcasts.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class,
                w!("Battery Status"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
            SetTimer(Some(hwnd), TICK_TIMER, TICK_MS, None);
            let exe_path = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    app,
                    rx,
                    hwnd,
                    hooks,
                    exe_path,
                    taskbar_created,
                    slots: HashMap::new(),
                    next_uid: 0,
                    blink_dim: false,
                })
            });
            Ok(Self { hwnd })
        }
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// Runs the message loop until "Exit". Returns the exit code.
    pub fn run(self) -> i32 {
        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        msg.wParam.0 as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_fits_and_terminates() {
        let mut buf = [1u16; 8];
        fill_wide(&mut buf, "abc");
        assert_eq!(&buf[..4], &[97, 98, 99, 0]);
    }

    #[test]
    fn wide_truncates_long_text() {
        let mut buf = [1u16; 4];
        fill_wide(&mut buf, "abcdef");
        assert_eq!(buf, [97, 98, 99, 0]);
    }

    #[test]
    fn guid_from_u128_round_trip() {
        let g = to_guid(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        assert_eq!(g.to_u128(), 0x0123_4567_89ab_cdef_0011_2233_4455_6677);
    }
}
