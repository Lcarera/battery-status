//! Device model, instance-ID parsers, and tray icon GUIDs. No Windows code.

/// Bluetooth address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Headphones,
    Phone,
    Gamepad,
    Generic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceState {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub percent: Option<u8>,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceEvent {
    Upsert(DeviceState),
    Removed(DeviceId),
    Resync(Vec<DeviceState>),
}

fn parse_hex12(s: &str) -> Option<DeviceId> {
    if s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        u64::from_str_radix(s, 16).ok().map(DeviceId)
    } else {
        None
    }
}

/// Extracts the Bluetooth address from a `BTHENUM\` or `BTHLE\` instance ID.
pub fn address_from_instance_id(id: &str) -> Option<DeviceId> {
    let upper = id.to_ascii_uppercase();
    let mut parts = upper.split('\\');
    let bus = parts.next()?;
    let second = parts.next()?;
    if bus != "BTHENUM" && bus != "BTHLE" {
        return None;
    }
    if let Some(rest) = second.strip_prefix("DEV_") {
        return parse_hex12(rest);
    }
    if bus == "BTHENUM" {
        let last = parts.next()?;
        let before = last.strip_suffix("_C00000000")?;
        return parse_hex12(before.get(before.len().checked_sub(12)?..)?);
    }
    None
}

pub fn classify(instance_id: &str, name: &str) -> DeviceKind {
    let id = instance_id.to_ascii_uppercase();
    if id.contains("{0000111E-") {
        return DeviceKind::Headphones;
    }
    if id.contains("{0000111F-") {
        return DeviceKind::Phone;
    }
    let n = name.to_lowercase();
    if n.contains("controller") || n.contains("gamepad") {
        DeviceKind::Gamepad
    } else {
        DeviceKind::Generic
    }
}

pub fn display_name(raw: &str) -> String {
    for suffix in [" Hands-Free AG", " Hands-Free HF", " Hands-Free"] {
        if let Some(s) = raw.strip_suffix(suffix) {
            return s.to_string();
        }
    }
    raw.to_string()
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Stable tray icon GUID: exe-path hash (high 64 bits) + address (low 64 bits).
pub fn icon_guid(id: DeviceId, exe_path: &str) -> u128 {
    let h = fnv1a64(exe_path.to_lowercase().as_bytes());
    ((h as u128) << 64) | id.0 as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADPHONES: &str = r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&1015\8&3223B969&0&5818627FF3D4_C00000000";
    const PHONE: &str = r"BTHENUM\{0000111F-0000-1000-8000-00805F9B34FB}_VID&00010075_PID&0100\8&3223B969&0&78C11DBB6C41_C00000000";
    const CONTROLLER: &str = r"BTHLE\DEV_408E2CAB5430\8&D3E38CA&0&408E2CAB5430";

    #[test]
    fn address_bthenum() {
        assert_eq!(address_from_instance_id(HEADPHONES), Some(DeviceId(0x5818627FF3D4)));
    }

    #[test]
    fn address_bthenum_dev_node() {
        assert_eq!(
            address_from_instance_id(r"BTHENUM\DEV_5818627FF3D4\9&1A2B3C4D&0&BLUETOOTHDEVICE_5818627FF3D4"),
            Some(DeviceId(0x5818627FF3D4))
        );
    }

    #[test]
    fn address_bthle() {
        assert_eq!(address_from_instance_id(CONTROLLER), Some(DeviceId(0x408E2CAB5430)));
    }

    #[test]
    fn address_other_prefix_none() {
        assert_eq!(address_from_instance_id(r"HID\VID_3434&PID_D030&MI_03\8&908D64D&0&0000"), None);
    }

    #[test]
    fn classify_headphones() {
        assert_eq!(classify(HEADPHONES, "1000X THE COLLEXION Hands-Free AG"), DeviceKind::Headphones);
    }

    #[test]
    fn classify_phone() {
        assert_eq!(classify(PHONE, "S26 de Luciano Hands-Free HF"), DeviceKind::Phone);
    }

    #[test]
    fn classify_gamepad() {
        assert_eq!(classify(CONTROLLER, "Xbox Wireless Controller"), DeviceKind::Gamepad);
    }

    #[test]
    fn classify_generic() {
        assert_eq!(classify(CONTROLLER, "Mouse"), DeviceKind::Generic);
    }

    #[test]
    fn display_name_strips_suffix() {
        assert_eq!(display_name("1000X THE COLLEXION Hands-Free AG"), "1000X THE COLLEXION");
        assert_eq!(display_name("S26 de Luciano Hands-Free HF"), "S26 de Luciano");
        assert_eq!(display_name("Xbox Wireless Controller"), "Xbox Wireless Controller");
    }

    #[test]
    fn guid_is_stable() {
        let p = r"C:\a\battery-status.exe";
        assert_eq!(icon_guid(DeviceId(1), p), icon_guid(DeviceId(1), p));
    }

    #[test]
    fn guid_differs_per_device() {
        let p = r"C:\a\battery-status.exe";
        assert_ne!(icon_guid(DeviceId(1), p), icon_guid(DeviceId(2), p));
    }

    #[test]
    fn guid_changes_with_exe_path() {
        assert_ne!(
            icon_guid(DeviceId(1), r"C:\a\battery-status.exe"),
            icon_guid(DeviceId(1), r"C:\b\battery-status.exe")
        );
    }
}
