# Keychron K8 HE battery over the 2.4 GHz dongle (spec 2.2)

Date: 2026-11-23

## Result

No documented method reads the battery level through the dongle.

## Firmware (Keychron/qmk_firmware, branches 2025q3 and hall_effect_playground)

- `keyboards/keychron/common/keychron_raw_hid.h`: the raw HID commands are 0xA0-0xAB
  (protocol, firmware, features, default layer, misc group, RGB, analog matrix,
  wireless DFU, factory test). The misc group (0xA7) has no battery sub-command.
- `common/wireless/lkbt51.c`: the keyboard MCU sends the level to its radio module
  with `LKBT51_CMD_UPDATE_BAT_LVL = 0x32` (and the state with 0x33).
  The module, not QMK, sends the level to the host.
- In Bluetooth mode the module most likely reports the level with the BLE Battery
  Service. If Windows shows it, v1 reads it from `BTHLE\DEV_<addr>` with no change.
  Not tested yet.
- The dongle firmware is closed. It is not in the repository.

## Dongle HID collections (USB VID_3434 PID_D030, read-only HidP_GetCaps)

| Interface | Usage page / usage | Note |
|---|---|---|
| MI_00 | 0x01 / 0x02 | Mouse |
| MI_01 COL01 | 0x8C / 0x01 | Report IDs 81-84, 177, 178. Feature length 1001. Dongle-private channel, likely DFU/pairing. |
| MI_01 COL02 | 0x01 / 0x04 | Joystick |
| MI_02 COL01-04 | 0x01 / 0x06, 0x0C / 0x01, 0x01 / 0x80 | Keyboard, consumer, system, NKRO |
| MI_03 | 0xFF60 / 0x61 | QMK raw HID, 32-byte reports (same as the wired keyboard) |

No collection uses the Battery System page (0x85) or the Generic Device battery
usage (0x06 / 0x20). No dongle node has the Windows battery DEVPKEY.

## Options

1. Use Bluetooth mode. If Windows shows the level, v1 shows it with no code change.
2. Keep "not available" for 2.4 GHz (current v1 state).
3. Listen passively to the 0x8C input reports (IDs 84, 177) and look for a byte that
   follows the battery level. This sends nothing to the device. The result is not certain.
