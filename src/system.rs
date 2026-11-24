//! Single instance (named mutex) and "Start with Windows" (HKCU Run key).

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::{HSTRING, w};

const MUTEX_NAME: &str = r"Local\BatteryStatus.SingleInstance";
const RUN_KEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: windows::core::PCWSTR = w!("BatteryStatus");

/// Holds the mutex. The next copy can start after this drops.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn single_instance_named(name: &str) -> Option<InstanceGuard> {
    unsafe {
        let handle = CreateMutexW(None, false, &HSTRING::from(name)).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(InstanceGuard(handle))
    }
}

/// `None` when another copy of the app runs.
pub fn single_instance() -> Option<InstanceGuard> {
    single_instance_named(MUTEX_NAME)
}

pub fn set_autostart(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
            let data: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32).ok()
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            if r.0 == 2 { Ok(()) } else { r.ok() } // 2 = ERROR_FILE_NOT_FOUND: already off
        }
    }
}

pub fn autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_refused() {
        let name = r"Local\BatteryStatus.Test.SingleInstance";
        let first = single_instance_named(name);
        assert!(first.is_some());
        assert!(single_instance_named(name).is_none());
        drop(first);
        assert!(single_instance_named(name).is_some());
    }
}
