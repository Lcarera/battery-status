# Battery Status

Battery Status shows the battery level of each connected Bluetooth device in the Windows tray.
It sends an alert when a device battery goes to 20% or less.

## Requirements

- Windows 11 (64-bit).
- No installer, no administrator rights, no other runtime.

## Install

1. Double-click `battery-status-setup-<version>.exe`.
2. If Windows shows "Windows protected your PC", click **More info**. Then click **Run anyway**.
   The setup has no code signature, so SmartScreen shows this message.
3. Follow the steps of the setup. It installs to `%LOCALAPPDATA%\Programs\battery-status\` and needs no administrator rights.

The setup adds Battery Status to the Start menu and to **Settings > Apps > Installed apps**.
To update, run the setup of the new version. It stops the running app before it replaces the files.

To use the app with no setup, put `battery-status.exe` in a permanent folder and double-click it.

The app has no window. It shows one tray icon for each connected device.
If you do not see the icons, open **Settings > Personalization > Taskbar > Other system tray icons** and turn them on.

The app starts with Windows by default. If you move the exe, start it one time from the new folder.
The app then updates the startup entry.

## Use

- Put the mouse on an icon to see the device name and the percentage.
- Right-click an icon to open the menu:
  - The list of connected devices with their percentages.
  - **Start with Windows** (on/off).
  - **Open settings file**.
  - **Exit**.
- When a device battery goes to 20%, 10%, and 5%, the app shows a toast and the icon blinks red.
  Click the icon or dismiss the toast to stop the blink.

To see alerts when "Do not disturb" is on, add Battery Status to the priority notifications list
(**Settings > System > Notifications > Set priority notifications**).

## Which devices show a battery level

The app reads the battery level that Windows reads.
If **Settings > Bluetooth & devices** shows a percentage for a device, the app shows it too.
If Windows shows no percentage, the app cannot show one.

## Settings

The settings file is `%APPDATA%\battery-status\config.toml`.
The app creates it at the first start. Use **Open settings file** in the menu to edit it.
The app reads the changes when you save the file.

```toml
start_with_windows = true

# Exact device names that get no icon.
ignore = ["WF-1000XM5"]

# Alert limits (percent). Rule: empty < critical < low < reset <= 100.
# An alert happens at low, critical, and empty. Alerts start again above reset.
[limits]
low = 20
critical = 10
empty = 5
reset = 25

# Bar colors: green above green_above, red at or below red_at_or_below, yellow between.
[bar_colors]
green_above = 50
red_at_or_below = 20

# Other limits for one device (use the name from the tooltip).
[devices."My Phone".limits]
low = 30
critical = 15
empty = 5
reset = 35
```

If the file is not valid, the app uses the default values and writes the error to the log.

The log file is `%APPDATA%\battery-status\battery-status.log`.

## Uninstall

1. Open **Settings > Apps > Installed apps**.
2. Find **Battery Status** and click **Uninstall**.
   The uninstaller stops the app and removes the startup entry.
3. To remove the settings and the log, delete the folder `%APPDATA%\battery-status\`.

If you installed with no setup:

1. Right-click an icon and turn off **Start with Windows**.
2. Click **Exit**.
3. Delete the exe and the folder `%APPDATA%\battery-status\`.

## Build from source

1. Install Rust (stable, MSVC toolchain).
2. Remove or change the `target-dir` line in `.cargo/config.toml`. It points to a folder on the author's PC.
3. Run `cargo build --release`.

The exe is in `target\release\battery-status.exe`.

To build the setup:

1. Install Inno Setup 6: `winget install JRSoftware.InnoSetup`.
2. Run `powershell -ExecutionPolicy Bypass -File installer\build.ps1`.

The setup is in `installer\Output\battery-status-setup-<version>.exe`.
`.cargo/config.toml` links the C runtime into the exe (`+crt-static`), so the exe needs no Visual C++ Redistributable.
