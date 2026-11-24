# Bluetooth connection property (Task 2 result)

Date: 2026-11-23

## Properties

- Battery: `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2` (byte, 0–100).
- Connected: `DEVPKEY_Device_IsConnected` = `{83DA6326-97A6-4088-9453-A1923F573B29} 15` (bool).

## Where each property is

| Device type | Battery node | Connected node |
|---|---|---|
| Bluetooth Classic (headphones, phone) | Profile node `BTHENUM\{<service UUID>}_VID...\..._<ADDRESS>_C00000000` | Parent node `BTHENUM\DEV_<ADDRESS>\...` |
| Bluetooth LE (Xbox controller) | `BTHLE\DEV_<ADDRESS>\...` | Same node |

On the Classic profile node, `IsConnected` is always `False`. Do not use it.
`DevNodeStatus` and `IsPresent` are the same for connected and disconnected devices. Do not use them.

## Values seen

| Address | Device | Battery (profile node) | IsConnected (DEV_ node) |
|---|---|---|---|
| 5818627FF3D4 | 1000X THE COLLEXION | 28 | True |
| 78C11DBB6C41 | S26 de Luciano | 80 | True |
| 408E2CAB5430 | Xbox Wireless Controller | 81 | True (on BTHLE node) |
| 8099E7D970B0 | WF-1000XM5 | 91 (old) | False |
| 5818621F4B8C | WH-1000XM6 | 90 (old) | False |

## Decision for Task 8

- Use `DeviceInformationKind::Device`.
- Request both properties for all nodes whose instance ID starts with `BTHENUM\` or `BTHLE\`.
- Join by Bluetooth address: battery from the node that has a battery value, connected state from the `DEV_` node.
- `address_from_instance_id` must also parse `BTHENUM\DEV_<ADDRESS>\...`.
