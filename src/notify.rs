//! Persistent toasts (scenario "reminder") with a Dismiss button.

use crate::alerts::AlertLevel;
use std::cell::RefCell;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationManager,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::core::{HSTRING, PCWSTR, w};

pub const AUMID: &str = "BatteryStatus.App";

/// Registers the AppUserModelID under HKCU so an unpackaged app can show toasts.
pub fn register_aumid() -> windows::core::Result<()> {
    let subkey = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    let value: Vec<u16> = "Battery Status".encode_utf16().chain(Some(0)).collect();
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            w!("DisplayName"),
            REG_SZ.0,
            Some(value.as_ptr().cast()),
            (value.len() * 2) as u32,
        )
        .ok()
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn toast_xml(name: &str, percent: u8, level: AlertLevel) -> String {
    let (what, advice) = match level {
        AlertLevel::Low => ("battery low", "Charge it soon."),
        AlertLevel::Critical => ("battery critical", "Charge it now."),
        AlertLevel::Empty => ("battery almost empty", "The device will turn off soon."),
    };
    format!(
        r#"<toast scenario="reminder"><visual><binding template="ToastGeneric"><text>{} {what}</text><text>{percent}% left. {advice}</text></binding></visual><actions><action activationType="system" arguments="dismiss" content="Dismiss"/></actions></toast>"#,
        escape(name)
    )
}

thread_local! {
    // Keep shown toasts alive so their Dismissed handlers stay registered.
    static SHOWN: RefCell<Vec<ToastNotification>> = const { RefCell::new(Vec::new()) };
}

pub fn show(
    name: &str,
    percent: u8,
    level: AlertLevel,
    on_dismiss: impl Fn() + Send + Sync + 'static,
) -> windows::core::Result<()> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(toast_xml(name, percent, level)))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, args| {
        if let Some(args) = args.as_ref() {
            if args.Reason()? == ToastDismissalReason::UserCanceled {
                on_dismiss();
            }
        }
        Ok(())
    }))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)?;
    SHOWN.with(|s| {
        let mut v = s.borrow_mut();
        v.push(toast);
        if v.len() > 20 {
            v.remove(0);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_has_reminder_scenario() {
        let x = toast_xml("S26", 20, AlertLevel::Low);
        assert!(x.contains(r#"scenario="reminder""#));
        assert!(x.contains(r#"arguments="dismiss""#));
    }

    #[test]
    fn xml_copy_per_level() {
        let low = toast_xml("S26", 20, AlertLevel::Low);
        assert!(low.contains("S26 battery low"));
        assert!(low.contains("20% left. Charge it soon."));
        assert!(toast_xml("S26", 10, AlertLevel::Critical).contains("battery critical"));
        assert!(toast_xml("S26", 5, AlertLevel::Empty).contains("battery almost empty"));
    }

    #[test]
    fn xml_escapes_name() {
        let x = toast_xml("A&B <x>", 20, AlertLevel::Low);
        assert!(x.contains("A&amp;B &lt;x&gt;"));
        let doc = windows::Data::Xml::Dom::XmlDocument::new().unwrap();
        doc.LoadXml(&windows::core::HSTRING::from(x)).unwrap();
    }
}
