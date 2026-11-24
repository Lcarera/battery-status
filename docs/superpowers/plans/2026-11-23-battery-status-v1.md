# Battery Status v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust tray app for Windows 11. It shows one tray icon for each connected Bluetooth device with a battery bar. It sends persistent alerts at 20%, 10%, and 5%.

**Architecture:** Pure modules (`config`, `alerts`, `device`, `icon`, `app`) hold all logic and have unit tests. The `app` module changes device events into a list of `Action` values. Thin Windows modules (`watcher`, `tray`, `notify`, `system`) do the actions. A WinRT device watcher sends events through a channel to the tray window thread, which runs the Win32 message loop.

**Tech Stack:** Rust (stable, MSVC target), `windows` crate, `serde` + `toml`, `notify` (file watch), `log` + `simplelog`.

**Spec:** `docs/spec.md`

## Global Constraints

- Platform: Windows 11 only. Target `x86_64-pc-windows-msvc`.
- No administrator rights at any point. Write only to `HKCU` and `%APPDATA%`.
- Battery property key: `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`.
- Do not enumerate all PnP devices. Query only Bluetooth devices (`BTHENUM\` and `BTHLE\` instance IDs).
- Alert limits: low 20, critical 10, empty 5. Reset above 25.
- Bar colors: green above 50, yellow 21–50, red 20 or less.
- Settings path: `%APPDATA%\battery-status\config.toml`. Log path: `%APPDATA%\battery-status\battery-status.log`.
- Backup poll: every 5 minutes.
- Release binary is one `.exe` with `#![windows_subsystem = "windows"]` (no console window).
- Before you use a crate API, get current docs with Context7 (`resolve-library-id`, then `query-docs`). Use the current crate versions from crates.io.
- Text in toasts, tooltips, and menus is English.

## Review Focus

1. **Explorer restarts** (crash or update). Windows removes all tray icons. Expected: the app adds all icons again on the `TaskbarCreated` message. Test: Task 9, step 5.
2. **The `.exe` moves to a different folder.** Windows binds a tray icon GUID to the `.exe` path, and `NIM_ADD` then fails. Expected: icons still appear. The GUID includes a hash of the `.exe` path. Test: Task 5, `guid_changes_with_exe_path`.
3. **A device connects but reports no battery value for a few seconds** (normal for Hands-Free headphones). Expected: no icon and no alert until a value arrives. No "0%" alert. Test: Task 7, `connected_without_value_has_no_icon`.
4. **The user saves an invalid settings file** (bad TOML or limits in the wrong order). Expected: the app keeps the last good settings, writes the error to the log, and does not stop. Test: Task 3 `invalid_limits_rejected` and Task 7 `reload_keeps_previous_on_error`.
5. **The level drops past more than one limit between two reads** (for example 25 → 8 after sleep). Expected: one alert at the most urgent level, not two toasts. Test: Task 4, `skip_to_critical_gives_one_alert`.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Crate setup and dependencies |
| `src/main.rs` | Entry point: logging, single instance, start the watcher and the tray loop |
| `src/config.rs` | `Config` type, defaults, load, validate, save |
| `src/alerts.rs` | Alert rules (spec 5.1). No Windows code |
| `src/device.rs` | `DeviceId`, `DeviceKind`, `DeviceState`, `DeviceEvent`, instance-ID parsers, icon GUID |
| `src/icon.rs` | Bar color and 16×16+ RGBA icon pixels. No Windows code |
| `src/app.rs` | App state. Changes events into `Action` values |
| `src/watcher.rs` | WinRT device watcher and backup poll. Sends `DeviceEvent` |
| `src/tray.rs` | Hidden window, message loop, tray icons, blink, menu |
| `src/notify.rs` | AppUserModelID registration, toast XML, toast show and dismiss |
| `src/system.rs` | Single-instance mutex, autostart `Run` key |
| `docs/notes/connection-property.md` | Result of the Task 2 check |

---

### Task 1: Toolchain and project setup

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `.gitignore`

**Interfaces:**
- Produces: a crate named `battery-status` that builds. `main.rs` declares all modules from the file table.

- [ ] **Step 1: Install Rust.** Run `winget install --id Rustlang.Rustup -e`. Open a new shell. Run `rustup default stable-x86_64-pc-windows-msvc`.
- [ ] **Step 2: Check the MSVC linker.** Run `cargo new --bin scratch-check` in the scratchpad, then `cargo build` in it. Expected: build succeeds. If it fails with `link.exe not found`, run `winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` and try again.
- [ ] **Step 3: Create the project.** In the project folder run `git init`, then `cargo init --name battery-status`. `.gitignore` contains `/target`.
- [ ] **Step 4: Add dependencies.** `windows` with the features that later tasks need (`Devices_Enumeration`, `Foundation`, `Foundation_Collections`, `Data_Xml_Dom`, `UI_Notifications`, `Win32_Foundation`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_System_Threading`, `Win32_System_Registry`, `Win32_System_LibraryLoader`, `Win32_System_Power`), `serde` (derive), `toml`, `notify`, `log`, `simplelog`. Dev-dependency: `tempfile`.
- [ ] **Step 5: Write `main.rs`.** Add `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`. Declare the modules as empty files. `main` only starts logging to the log path.
- [ ] **Step 6: Verify.** Run `cargo build` and `cargo test`. Expected: both succeed with 0 tests.
- [ ] **Step 7: Commit.** `git add -A && git commit -m "chore: set up Rust project"`

---

### Task 2: Check how Windows reports "connected"

The spec needs connected devices only. The battery property stays on disconnected devices (verified: WF-1000XM5 and WH-1000XM6 keep old values). This task finds the property that tells the connection state.

**Files:**
- Create: `docs/notes/connection-property.md`

**Interfaces:**
- Produces: the name of the connection property and the `DeviceInformationKind` to use. Task 8 uses this result.

- [ ] **Step 1: Read candidate properties.** For the five Bluetooth device nodes from the spec, read these properties with `Get-PnpDeviceProperty -InstanceId <id> -KeyName <key>`:
  - `DEVPKEY_Device_IsConnected` (`{83DA6326-97A6-4088-9453-A1923F573B29} 15`)
  - `DEVPKEY_Device_DevNodeStatus` (`{4340A6C5-93FA-4706-972C-7B648008A5A7} 2`)
  - `DEVPKEY_Device_IsPresent` (`{540B947E-8B40-45BC-A8A2-6A0B894CBDA2} 5`)
- [ ] **Step 2: Compare.** Find the property that is different for connected devices (1000X THE COLLEXION, S26, Xbox controller) and disconnected devices (WF-1000XM5, WH-1000XM6).
- [ ] **Step 3: Test the change.** Turn off the Xbox controller. Read the property again. Expected: the value changes in less than 10 seconds.
- [ ] **Step 4: Fallback.** If no device-node property works, check the AssociationEndpoint property `System.Devices.Aep.IsConnected` for the same devices. Record the Bluetooth address mapping.
- [ ] **Step 5: Record and commit.** Write the property key, the values seen, and the chosen `DeviceInformationKind` in `docs/notes/connection-property.md`. Commit: `docs: record Bluetooth connection property`.

---

### Task 3: Settings (`config`)

**Files:**
- Create: `src/config.rs` (tests in the same file, `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct Limits { pub low: u8, pub critical: u8, pub empty: u8, pub reset: u8 }`, `Default` = 20 / 10 / 5 / 25
  - `pub struct BarColors { pub green_above: u8, pub red_at_or_below: u8 }`, `Default` = 50 / 20
  - `pub struct DeviceOverride { pub limits: Option<Limits> }`
  - `pub struct Config { pub limits: Limits, pub bar_colors: BarColors, pub ignore: Vec<String>, pub devices: HashMap<String, DeviceOverride>, pub start_with_windows: bool }`, `Default` has `start_with_windows = true`
  - `impl Config { pub fn limits_for(&self, device_name: &str) -> Limits; pub fn is_ignored(&self, device_name: &str) -> bool; pub fn validate(&self) -> Result<(), String> }`
  - `pub fn parse(text: &str) -> Result<Config, String>` (parse and validate)
  - `pub fn load_or_create(path: &Path) -> Result<(Config, bool), String>` (bool = the file was created now)
  - `pub fn save(path: &Path, cfg: &Config) -> Result<(), String>`
  - `pub fn default_path() -> PathBuf` (`%APPDATA%\battery-status\config.toml`)
- All struct fields use `#[serde(default)]`, so a partial file is valid. `devices` keys are display names (Task 5 `display_name`).

- [ ] **Step 1: Write the failing tests.**
  - `empty_file_gives_defaults`: `parse("")` gives `limits` 20/10/5/25, bar colors 50/20, `start_with_windows == true`.
  - `partial_file_keeps_other_defaults`: `parse("[limits]\nlow = 30")` gives `low == 30`, `critical == 10`.
  - `invalid_limits_rejected`: `parse("[limits]\nlow = 10\ncritical = 20")` is `Err`. Rule: `empty < critical < low < reset <= 100`.
  - `bad_toml_rejected`: `parse("[limits")` is `Err`.
  - `override_used_for_named_device`: with `[devices."S26 de Luciano".limits]` low 30 / critical 15 / empty 5 / reset 35, `limits_for("S26 de Luciano").low == 30` and `limits_for("Other").low == 20`.
  - `ignore_matches_exact_name`: `ignore = ["WF-1000XM5"]` gives `is_ignored("WF-1000XM5") == true`, `is_ignored("WH-1000XM6") == false`.
  - `load_creates_file_when_missing`: in a `tempfile` directory, `load_or_create` returns defaults and `true`, and the file exists. A second call returns `false`.
- [ ] **Step 2: Run the tests.** `cargo test config`. Expected: FAIL (items not defined).
- [ ] **Step 3: Implement** the interface above with `serde` and `toml`. `load_or_create` creates the parent folder.
- [ ] **Step 4: Run the tests.** `cargo test config`. Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add settings file`

---

### Task 4: Alert rules (`alerts`)

**Files:**
- Create: `src/alerts.rs`

**Interfaces:**
- Consumes: `config::Limits`
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum AlertLevel { Low, Critical, Empty }`
  - `pub struct AlertTracker` with `pub fn new() -> Self`
  - `pub fn update(&mut self, key: &str, percent: u8, limits: &Limits) -> Option<AlertLevel>` (returns a level only when an alert must be sent now)
  - `pub fn forget(&mut self, key: &str)` (call on disconnect)
- State per key: the most urgent level already alerted (or none). Algorithm: find the current level from `percent` (`<= empty` → Empty, `<= critical` → Critical, `<= low` → Low). If `percent > reset`, clear the state and return `None`. If the current level is more urgent than the stored level, store it and return it. Else return `None`.

- [ ] **Step 1: Write the failing tests** (all with default limits):
  - `above_low_no_alert`: update 50 → `None`.
  - `crossing_low_alerts_once`: 21 → `None`, 20 → `Some(Low)`, 19 → `None`, 20 → `None`.
  - `critical_and_empty_alert`: 20 → `Low`, 10 → `Critical`, 5 → `Empty`, 3 → `None`.
  - `skip_to_critical_gives_one_alert`: 25 → `None`, 8 → `Some(Critical)`, 7 → `None`.
  - `no_reset_between_21_and_25`: 20 → `Low`, 24 → `None`, 20 → `None`.
  - `reset_above_25`: 20 → `Low`, 26 → `None`, 20 → `Some(Low)`.
  - `first_value_low_alerts`: first update 15 → `Some(Low)` (spec rule 4).
  - `forget_allows_alert_on_reconnect`: 15 → `Low`, `forget`, 15 → `Some(Low)`.
  - `keys_are_independent`: "a" 15 → `Low`, "b" 15 → `Low`.
  - `zero_percent_is_empty`: 0 → `Some(Empty)`.
- [ ] **Step 2: Run the tests.** `cargo test alerts`. Expected: FAIL.
- [ ] **Step 3: Implement** the interface above.
- [ ] **Step 4: Run the tests.** `cargo test alerts`. Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add alert rules`

---

### Task 5: Device model and parsers (`device`)

**Files:**
- Create: `src/device.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct DeviceId(pub u64)` (Bluetooth address)
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DeviceKind { Headphones, Phone, Gamepad, Generic }`
  - `#[derive(Debug, Clone, PartialEq)] pub struct DeviceState { pub id: DeviceId, pub name: String, pub kind: DeviceKind, pub percent: Option<u8>, pub connected: bool }`
  - `pub enum DeviceEvent { Upsert(DeviceState), Removed(DeviceId), Resync(Vec<DeviceState>) }`
  - `pub fn address_from_instance_id(id: &str) -> Option<DeviceId>`
  - `pub fn classify(instance_id: &str, name: &str) -> DeviceKind`
  - `pub fn display_name(raw: &str) -> String`
  - `pub fn icon_guid(id: DeviceId, exe_path: &str) -> u128`
- Rules:
  - Address: the last 12 hex characters before `_C00000000` for `BTHENUM\`, or the 12 hex characters after `DEV_` for `BTHLE\`. Other prefixes → `None`.
  - Kind: service `0000111E` → Headphones. Service `0000111F` → Phone. Name contains "controller" or "gamepad" (any case) → Gamepad. Else Generic.
  - Display name: remove the suffix ` Hands-Free AG`, ` Hands-Free HF`, or ` Hands-Free`.
  - GUID: FNV-1a 64-bit hash of the lowercase `exe_path` in the high 64 bits, the address in the low 64 bits. Fixed, no randomness.

- [ ] **Step 1: Write the failing tests.** Use the real instance IDs from the spec scan:
  - `address_bthenum`: `BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&1015\8&3223B969&0&5818627FF3D4_C00000000` → `DeviceId(0x5818627FF3D4)`.
  - `address_bthle`: `BTHLE\DEV_408E2CAB5430\8&D3E38CA&0&408E2CAB5430` → `DeviceId(0x408E2CAB5430)`.
  - `address_other_prefix_none`: `HID\VID_3434&PID_D030&MI_03\8&908D64D&0&0000` → `None`.
  - `classify_headphones`, `classify_phone` (the S26 ID with `0000111F`), `classify_gamepad` (`BTHLE\...` with "Xbox Wireless Controller"), `classify_generic` (`BTHLE\...` with "Mouse").
  - `display_name_strips_suffix`: "1000X THE COLLEXION Hands-Free AG" → "1000X THE COLLEXION". "S26 de Luciano Hands-Free HF" → "S26 de Luciano". "Xbox Wireless Controller" stays the same.
  - `guid_is_stable`: same inputs give the same value two times.
  - `guid_differs_per_device`: two addresses give different values.
  - `guid_changes_with_exe_path`: same address, paths `C:\a\battery-status.exe` and `C:\b\battery-status.exe` give different values.
- [ ] **Step 2: Run the tests.** `cargo test device`. Expected: FAIL.
- [ ] **Step 3: Implement** the interface above.
- [ ] **Step 4: Run the tests.** `cargo test device`. Expected: PASS.
- [ ] **Step 5: Commit.** `feat: add device model and parsers`

---

### Task 6: Icon pixels (`icon`)

**Files:**
- Create: `src/icon.rs`

**Interfaces:**
- Consumes: `device::DeviceKind`, `config::BarColors`
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum BarColor { Green, Yellow, Red }`
  - `pub fn bar_color(percent: u8, colors: &BarColors) -> BarColor`
  - `pub fn render(kind: DeviceKind, percent: u8, color: BarColor, size: u32, dimmed: bool) -> Vec<u32>` (ARGB, `size * size` pixels, row-major)
- Layout at any `size`: the device symbol fills the left part (about 60% of the width). A vertical battery bar fills the right part (about 30% of the width), from the bottom up, with height `percent * bar_height / 100`. Draw the symbols as simple shapes scaled from a 16×16 grid (no font, no image files). `dimmed = true` makes the bar transparent; Task 9 uses it for the blink.
- Colors: Green `0xFF2EB82E`, Yellow `0xFFE6B800`, Red `0xFFE53935`. Symbol: white `0xFFFFFFFF`.

- [ ] **Step 1: Write the failing tests:**
  - `color_limits`: 51 → Green, 50 → Yellow, 21 → Yellow, 20 → Red, 0 → Red (default `BarColors`).
  - `render_size`: `render(.., 16, ..).len() == 256` and `render(.., 24, ..).len() == 576`.
  - `bar_height_follows_percent`: count pixels with the Green value at size 16. The count for 100% is more than the count for 50%, and the count for 0% is 0.
  - `bar_uses_color`: at 15% with `Red`, at least one pixel is `0xFFE53935` and no pixel is green.
  - `dimmed_has_no_bar_pixels`: `dimmed = true` gives no pixel of the bar color.
  - `kinds_render_differently`: Headphones and Phone at the same percent give different pixel vectors.
- [ ] **Step 2: Run the tests.** `cargo test icon`. Expected: FAIL.
- [ ] **Step 3: Implement** the interface above.
- [ ] **Step 4: Run the tests.** `cargo test icon`. Expected: PASS.
- [ ] **Step 5: Commit.** `feat: render tray icon pixels`

---

### Task 7: App state and actions (`app`)

**Files:**
- Create: `src/app.rs`

**Interfaces:**
- Consumes: Tasks 3–6.
- Produces:
  - `pub enum Action { ShowIcon { id: DeviceId, kind: DeviceKind, percent: u8, color: BarColor, tooltip: String }, RemoveIcon(DeviceId), StartBlink(DeviceId), StopBlink(DeviceId), ShowToast { id: DeviceId, name: String, percent: u8, level: AlertLevel } }`
  - `pub struct App` with `pub fn new(config: Config) -> Self`
  - `pub fn handle(&mut self, event: DeviceEvent) -> Vec<Action>`
  - `pub fn acknowledge(&mut self, id: DeviceId) -> Vec<Action>` (user clicked the icon or dismissed the toast)
  - `pub fn reload_config(&mut self, text: &str) -> Result<Vec<Action>, String>`
  - `pub fn menu_items(&self) -> Vec<String>` (for example `"1000X THE COLLEXION — 34%"`, sorted by name)
- Rules:
  - A device is visible when `connected && percent.is_some() && !config.is_ignored(name)`.
  - Visible and new or changed → `ShowIcon`. Was visible and now not → `RemoveIcon` and `AlertTracker::forget`.
  - Tooltip format: `"{name} — {percent}%"`.
  - An alert from `AlertTracker::update` (key = display name, limits = `limits_for(name)`) → `ShowToast` and `StartBlink`.
  - `acknowledge` → `StopBlink` only if the device blinks now.
  - `Resync(list)`: compare to the current state. Remove devices not in the list. Update the others with the same rules.
  - `reload_config`: on error, keep the old config and return `Err`. On success, apply the new config to all known devices (ignored devices lose their icon).

- [ ] **Step 1: Write the failing tests** (build `DeviceState` values with a small helper):
  - `connected_with_value_shows_icon`: Upsert connected 34% → one `ShowIcon` with tooltip `"1000X THE COLLEXION — 34%"` and `Yellow`.
  - `connected_without_value_has_no_icon`: Upsert connected `percent: None` → no actions.
  - `disconnected_has_no_icon`: Upsert `connected: false`, 90% → no actions.
  - `disconnect_removes_icon`: show, then Upsert `connected: false` → `RemoveIcon`.
  - `low_value_toasts_and_blinks`: Upsert 20% → `ShowIcon`, `ShowToast { level: Low, .. }`, `StartBlink`.
  - `acknowledge_stops_blink_once`: after the alert, `acknowledge` → `[StopBlink]`. Second call → `[]`.
  - `ignored_device_hidden`: config `ignore = ["S26 de Luciano"]`, Upsert S26 20% → no actions.
  - `resync_removes_missing`: show two devices, `Resync` with one → `RemoveIcon` for the other.
  - `reload_keeps_previous_on_error`: `reload_config("[limits")` → `Err`. A next Upsert at 20% still toasts with the old limits.
  - `reload_hides_newly_ignored`: show S26, reload with `ignore = ["S26 de Luciano"]` → `RemoveIcon`.
  - `menu_items_sorted`: two visible devices give two items in name order.
- [ ] **Step 2: Run the tests.** `cargo test app`. Expected: FAIL.
- [ ] **Step 3: Implement** the interface above.
- [ ] **Step 4: Run the tests.** `cargo test`. Expected: all tests PASS.
- [ ] **Step 5: Commit.** `feat: add app state and actions`

---

### Task 8: Windows device watcher (`watcher`)

**Files:**
- Create: `src/watcher.rs`
- Modify: `src/main.rs` (temporary debug run, see step 3)

**Interfaces:**
- Consumes: `device::*`, `docs/notes/connection-property.md`.
- Produces: `pub fn start(on_event: impl Fn(DeviceEvent) + Send + Sync + 'static) -> windows::core::Result<WatcherHandle>`. `WatcherHandle` keeps the watcher alive and has `pub fn resync(&self)`, which reads all devices again and sends `DeviceEvent::Resync`.
- Approach: `DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties` with the kind from Task 2, an AQS filter for Bluetooth instance IDs only, and the battery and connection properties. `Added` and `Updated` send `Upsert`. `Removed` sends `Removed`. A timer thread calls `resync` every 300 s. Parse address, kind, and name with Task 5 functions. Skip devices where `address_from_instance_id` is `None`.
- Battery value type: the property holds a byte (`u8`). Clamp values above 100 to 100.

- [ ] **Step 1: Get docs.** Use Context7 for the `windows` crate `DeviceWatcher` and `DeviceInformationUpdate` APIs.
- [ ] **Step 2: Implement** `start` and `resync`.
- [ ] **Step 3: Manual check.** Temporarily make `main` print each event with `log::info!` and run `cargo run`. Expected in the log: Upsert events for 1000X THE COLLEXION 34%-ish, S26, and Xbox controller with `connected: true`, and WF-1000XM5 / WH-1000XM6 with `connected: false`. The first `Resync` arrives in less than 5 s.
- [ ] **Step 4: Manual check of changes.** Turn off the Xbox controller. Expected: an Upsert with `connected: false` in less than 10 s. Turn it on again. Expected: `connected: true`.
- [ ] **Step 5: Check speed.** Expected: the first full read takes less than 5 s (the PnP scan took more than 2 minutes).
- [ ] **Step 6: Commit.** Remove the temporary print code. Commit: `feat: add Bluetooth device watcher`.

---

### Task 9: Tray icons, blink, and menu (`tray`)

**Files:**
- Create: `src/tray.rs`

**Interfaces:**
- Consumes: `app::{App, Action}`, `icon::render`, `device::icon_guid`.
- Produces:
  - `pub const WM_APP_EVENT: u32 = WM_APP + 1` (posted by other threads after they queue a `DeviceEvent`)
  - `pub struct Tray` with `pub fn new(app: App, events: Receiver<DeviceEvent>) -> windows::core::Result<Self>` and `pub fn run(self) -> i32` (message loop, returns the exit code)
  - `pub fn post_wakeup(hwnd: isize)` (thread-safe; other threads call it after a channel send)
  - `pub fn apply(&mut self, actions: Vec<Action>)`
- Approach:
  - One hidden message-only window. Register the `TaskbarCreated` message with `RegisterWindowMessageW`.
  - `ShowIcon`: `Shell_NotifyIconW` with `NIF_GUID | NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP`, `NOTIFYICON_VERSION_4`. Use `NIM_ADD` for a new icon and `NIM_MODIFY` for a known icon. Make the `HICON` from `icon::render` at `GetSystemMetrics(SM_CXSMICON)` size. Destroy the old `HICON`.
  - If `NIM_ADD` fails, call `NIM_DELETE` for the same GUID, then try `NIM_ADD` one more time. Log a failure.
  - Blink: one `SetTimer` of 500 ms for all blinking icons. Each tick changes between `dimmed = false` and `dimmed = true`.
  - Left click (`NIN_SELECT` / `WM_LBUTTONUP`) → `App::acknowledge`. Right click (`WM_CONTEXTMENU`) → menu: `App::menu_items` (disabled lines), separator, "Start with Windows" (check mark), "Open settings file", "Exit".
  - `TaskbarCreated` → add all current icons again.
  - `WM_POWERBROADCAST` with `PBT_APMRESUMEAUTOMATIC` → call the watcher `resync` (pass the `WatcherHandle` through a callback set in Task 11).
  - "Exit" → `NIM_DELETE` all icons, then `PostQuitMessage(0)`.

- [ ] **Step 1: Get docs.** Use Context7 for `Shell_NotifyIconW`, `NOTIFYICONDATAW`, and `CreateIconIndirect` in the `windows` crate.
- [ ] **Step 2: Implement** the interface above. The menu items "Start with Windows" and "Open settings file" call functions from Task 11 through callbacks. Use log lines for them until Task 11.
- [ ] **Step 3: Manual check of icons.** Connect `watcher` → channel → `Tray` in `main` and run `cargo run`. Expected: three icons (headphones, phone, gamepad), with correct bar colors. The tooltip shows name and percentage.
- [ ] **Step 4: Manual check of the GUID.** Drag the three icons from the overflow menu to the visible area. Stop the app and start it again. Expected: the icons are still in the visible area.
- [ ] **Step 5: Manual check of Explorer restart.** Run `Stop-Process -Name explorer` (Windows starts Explorer again). Expected: the three icons come back in less than 5 s.
- [ ] **Step 6: Manual check of blink.** Set `[limits] low = 95, critical = 94, empty = 93, reset = 96` in the settings file. Expected: the icons blink. Left click on one icon. Expected: that icon stops the blink and stays red.
- [ ] **Step 7: Commit.** `feat: add tray icons, blink, and menu`

---

### Task 10: Toasts (`notify`)

**Files:**
- Create: `src/notify.rs`

**Interfaces:**
- Consumes: `alerts::AlertLevel`, `device::DeviceId`.
- Produces:
  - `pub const AUMID: &str = "BatteryStatus.App"`
  - `pub fn register_aumid() -> windows::core::Result<()>` (writes `HKCU\Software\Classes\AppUserModelId\BatteryStatus.App` with `DisplayName = "Battery Status"`)
  - `pub fn toast_xml(name: &str, percent: u8, level: AlertLevel) -> String`
  - `pub fn show(name: &str, percent: u8, level: AlertLevel, on_dismiss: impl Fn() + Send + Sync + 'static) -> windows::core::Result<()>`
- Copy (exact text):
  - Low: title `"{name} battery low"`, body `"{percent}% left. Charge it soon."`
  - Critical: title `"{name} battery critical"`, body `"{percent}% left. Charge it now."`
  - Empty: title `"{name} battery almost empty"`, body `"{percent}% left. The device will turn off soon."`
- XML: `<toast scenario="reminder">`, one `<action activationType="system" arguments="dismiss" content="Dismiss"/>`. Escape `&`, `<`, `>`, `"` in the name. `show` calls `on_dismiss` from the toast `Dismissed` event. Task 11 makes `on_dismiss` post an acknowledge message to the tray window.

- [ ] **Step 1: Write the failing tests:**
  - `xml_has_reminder_scenario`: `toast_xml("S26", 20, Low)` contains `scenario="reminder"` and `arguments="dismiss"`.
  - `xml_copy_per_level`: Low contains `"S26 battery low"` and `"20% left. Charge it soon."`. Critical contains `"battery critical"`. Empty contains `"battery almost empty"`.
  - `xml_escapes_name`: `toast_xml("A&B <x>", 20, Low)` contains `A&amp;B &lt;x&gt;` and parses with `windows::Data::Xml::Dom::XmlDocument::LoadXml` without error.
- [ ] **Step 2: Run the tests.** `cargo test notify`. Expected: FAIL.
- [ ] **Step 3: Implement** the interface above. Get docs for `ToastNotificationManager::CreateToastNotifierWithId` with Context7.
- [ ] **Step 4: Run the tests.** `cargo test notify`. Expected: PASS.
- [ ] **Step 5: Manual check.** With the test limits from Task 9 step 6, run the app. Expected: one toast for each device. Each toast stays on screen until you click "Dismiss". After "Dismiss", the icon of that device stops the blink.
- [ ] **Step 6: Commit.** `feat: add persistent toasts`

---

### Task 11: System integration and final wiring (`system`, `main`)

**Files:**
- Create: `src/system.rs`
- Modify: `src/main.rs`, `src/tray.rs` (connect callbacks)

**Interfaces:**
- Produces:
  - `pub struct InstanceGuard` and `pub fn single_instance() -> Option<InstanceGuard>` (named mutex `Local\BatteryStatus.SingleInstance`; `None` when another copy runs)
  - `pub fn set_autostart(enabled: bool) -> windows::core::Result<()>` (value `BatteryStatus` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, data = quoted current `.exe` path)
  - `pub fn autostart_enabled() -> bool`
- `main` order:
  1. Start logging to the log path.
  2. Call `single_instance()`. Exit with code 0 when it gives `None`.
  3. Call `config::load_or_create`. If the file was created now and `start_with_windows` is true, call `set_autostart(true)`. If the file is invalid, log the error and use defaults.
  4. Call `notify::register_aumid()`.
  5. Create the channel, `App`, and `Tray`. Start the `watcher` with a callback that sends to the channel and calls `post_wakeup`.
  6. Start a `notify` file watcher on the settings folder. On change of `config.toml`, read the file and call `App::reload_config` on the tray thread (send through the same wakeup path). Log errors.
  7. Run `Tray::run`.
- Menu callbacks: "Start with Windows" toggles `set_autostart`, and saves `start_with_windows` in the settings file. "Open settings file" opens it with `ShellExecuteW` and the `open` verb.

- [ ] **Step 1: Write the failing test:** `second_instance_refused`: the first `single_instance()` is `Some`. A second call in the same process while the first guard lives is `None`.
- [ ] **Step 2: Run the test.** `cargo test system`. Expected: FAIL.
- [ ] **Step 3: Implement** `system.rs` and the `main` order above.
- [ ] **Step 4: Run all tests.** `cargo test`. Expected: PASS.
- [ ] **Step 5: Build release.** `cargo build --release`. Expected: `target\release\battery-status.exe` exists. It opens no console window.
- [ ] **Step 6: Manual end-to-end check.** Run the release `.exe` and check each item:
  - Three icons appear with correct values.
  - A second start of the `.exe` exits, and no extra icons appear.
  - The value `BatteryStatus` exists in the `Run` key. The menu item "Start with Windows" removes it and adds it again.
  - "Open settings file" opens `config.toml`. Save a change to `ignore = ["Xbox Wireless Controller"]`. Expected: the controller icon goes away in less than 3 s. Remove the change. Expected: the icon comes back.
  - Save invalid TOML. Expected: no crash, and the log file has the error.
  - Put the PC to sleep and wake it. Expected: icons show current values in less than 10 s, and no old alerts appear.
- [ ] **Step 7: Commit.** `feat: wire app, autostart, and single instance`

---

## Spec coverage

| Spec section | Task |
|---|---|
| 2.1 Battery source, Bluetooth-only query | 8 (and Global Constraints) |
| 2.2 Keyboard | Out of scope (spec 11) |
| 3 Device detection, resume, active devices | 2, 7, 8, 9 |
| 4 Tray icons, GUID, colors, tooltip | 5, 6, 7, 9 |
| 5.1 Alert rules | 4, 7 |
| 5.2 Toast and blink | 9, 10 |
| 6 Mouse actions | 9, 11 |
| 7 Settings, reload, log | 1, 3, 7, 11 |
| 8 Autostart, single instance, AUMID | 10, 11 |
| 9 Technology | 1, 8, 9 |
| 10 Structure and tests | All |
