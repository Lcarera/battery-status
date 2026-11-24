# Battery Status — Design Spec (v1)

Date: 2026-11-23

## 1. Goal

Show the battery level of each connected wireless device in the Windows 11 tray.
Send clear alerts when a device battery goes to 20% or less.

## 2. Devices

| Device | Connection | Battery source | v1 status |
|---|---|---|---|
| Sony 1000X THE COLLEXION | Bluetooth Classic (Hands-Free AG) | Windows device property | Supported |
| Samsung S26 | Bluetooth Classic (Hands-Free HF) | Windows device property | Supported |
| Xbox Wireless Controller | Bluetooth LE | Windows device property | Supported |
| Other paired Bluetooth devices (WF-1000XM5, WH-1000XM6, ...) | Bluetooth | Windows device property | Supported when connected |
| Keychron K8 HE | 2.4 GHz dongle (`VID_3434&PID_D030`) | None known | Not supported in v1 |

### 2.1 Battery data source

- Windows stores the battery level in the device property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`.
- Verified on 2026-11-23: the property contains the same values that Windows Settings shows.
- Do not scan all PnP devices. A full scan took more than 2 minutes. Query only Bluetooth devices.

### 2.2 Keyboard (separate research task)

- Keychron Launcher does not show the battery level over 2.4 GHz.
- Research task: search the Keychron QMK firmware for a battery command on the vendor HID interface (`MI_03`).
- If the command exists, add the keyboard in a later version. The keyboard gets no icon in v1.

## 3. Device detection

- Use a Windows device watcher (`Windows.Devices.Enumeration`) with the battery property and the connection property.
- The watcher reports connect, disconnect, and battery changes.
- A backup check reads all devices every 5 minutes.
- After the PC wakes up from sleep, read all devices again.
- A device is "active" only when it is connected and has a battery value. Ignore old values of disconnected devices.

## 4. Tray icons

- One tray icon for each active device.
- The icon appears when the device connects. The icon goes away when the device disconnects.
- Each icon has a stable GUID, made from the Bluetooth address. Windows then remembers if the icon is visible.
- Icon size: 16 × 16 px at 100% scale. Support higher DPI.
- Icon content: a device symbol (headphones, phone, gamepad, generic) and a battery bar.
- Bar colors (default):
  - Green: above 50%
  - Yellow: 21% to 50%
  - Red: 20% or less
- Tooltip: device name and percentage, for example `1000X THE COLLEXION — 34%`.

## 5. Alerts

### 5.1 Rules

1. Alert one time when a device goes to 20% or less.
2. Alert again at 10% and at 5%. These alerts are more urgent.
3. Reset the alerts for a device only when its level goes above 25%.
4. If a device connects with 20% or less, alert immediately.
5. All devices use the same limits. The settings file can set other limits for each device.

### 5.2 Alert signals

- A persistent toast (scenario `reminder`) with a "Dismiss" button. The toast stays until the user closes it.
- The tray icon of the device blinks red.
- The blink stops when the user clicks the icon or dismisses the toast. The icon stays red.
- Setup note: add the app to the Windows priority notifications list, so that "Do not disturb" does not hide the toast.

## 6. Mouse actions

| Action | Result |
|---|---|
| Mouse over icon | Tooltip with name and percentage |
| Left click on blinking icon | Stop the blink |
| Left click on normal icon | Nothing |
| Right click on any icon | Menu: list of connected devices with percentages, "Start with Windows" (on/off), "Open settings file", "Exit" |

No separate panel window in v1.

## 7. Settings

- File: `%APPDATA%\battery-status\config.toml`.
- Contents: alert limits (20 / 10 / 5), reset limit (25), bar color limits, per-device limits, list of devices to ignore.
- The app reads the file again when the user saves it.
- Log file: `%APPDATA%\battery-status\battery-status.log`.

## 8. App behavior

- Start with Windows: on by default. Use `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- Single instance: a second copy stops immediately (named mutex).
- Toast registration: register an AppUserModelID under `HKCU`. No administrator rights.

## 9. Technology

- Language: Rust.
- `windows` crate for the device watcher, the tray API, and the toasts.
- Call `Shell_NotifyIconW` directly with `NIF_GUID`. Common tray crates do not support a fixed GUID for each icon.
- Output: one `.exe` file.

## 10. Structure and tests

- `alerts` module: the alert rules (section 5.1). No Windows code. Unit tests cover all rules.
- `devices` module: reads devices behind a trait. Tests use fake devices.
- `tray` module: icons, tooltips, menu, blink.
- `notify` module: toasts.
- `config` module: settings file and reload.

## 11. Out of scope for v1

- Keychron K8 HE battery (see section 2.2).
- A separate panel window.
- An installer.
