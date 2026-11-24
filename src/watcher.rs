//! Bluetooth device watcher. `NodeTable` (pure) joins device nodes by address;
//! the WinRT part feeds it and sends `DeviceEvent`s.

use crate::device::{self, DeviceEvent, DeviceId, DeviceState};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::{IPropertyValue, TypedEventHandler};
use windows::core::{HSTRING, Interface};

pub const BATTERY_KEY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
pub const CONNECTED_KEY: &str = "{83DA6326-97A6-4088-9453-A1923F573B29} 15";
const AQS: &str = r#"System.Devices.DeviceInstanceId:~<"BTHENUM" OR System.Devices.DeviceInstanceId:~<"BTHLE""#;
const POLL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq)]
pub struct NodeInfo {
    pub instance_id: String,
    pub name: String,
    pub battery: Option<u8>,
    pub connected: Option<bool>,
}

impl NodeInfo {
    /// `BTHENUM\DEV_...` or `BTHLE\DEV_...`: the node that holds the real connection state.
    fn is_dev_node(&self) -> bool {
        self.instance_id.split('\\').nth(1).is_some_and(|s| s.to_ascii_uppercase().starts_with("DEV_"))
    }
}

#[derive(Debug, Default)]
pub struct NodeTable {
    nodes: HashMap<String, NodeInfo>,
}

impl NodeTable {
    pub fn upsert(&mut self, node: NodeInfo) -> Option<DeviceEvent> {
        let addr = device::address_from_instance_id(&node.instance_id)?;
        self.nodes.insert(node.instance_id.clone(), node);
        self.device_for(addr).map(DeviceEvent::Upsert)
    }

    pub fn remove(&mut self, instance_id: &str) -> Option<DeviceEvent> {
        self.nodes.remove(instance_id)?;
        let addr = device::address_from_instance_id(instance_id)?;
        Some(match self.device_for(addr) {
            Some(d) => DeviceEvent::Upsert(d),
            None => DeviceEvent::Removed(addr),
        })
    }

    /// Replaces the table with `fresh`, but keeps the live entry (or absence)
    /// of each node in `changed`.
    pub fn merge_fresh(&mut self, fresh: NodeTable, changed: &HashSet<String>) {
        let mut nodes = fresh.nodes;
        for id in changed {
            match self.nodes.remove(id) {
                Some(live) => nodes.insert(id.clone(), live),
                None => nodes.remove(id),
            };
        }
        self.nodes = nodes;
    }

    pub fn snapshot(&self) -> Vec<DeviceState> {
        let addrs: BTreeSet<DeviceId> =
            self.nodes.keys().filter_map(|id| device::address_from_instance_id(id)).collect();
        addrs.into_iter().filter_map(|a| self.device_for(a)).collect()
    }

    fn device_for(&self, addr: DeviceId) -> Option<DeviceState> {
        let mut nodes: Vec<&NodeInfo> = self
            .nodes
            .values()
            .filter(|n| device::address_from_instance_id(&n.instance_id) == Some(addr))
            .collect();
        if nodes.is_empty() {
            return None;
        }
        nodes.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        let battery_node = nodes.iter().find(|n| n.battery.is_some()).copied();
        let dev_node = nodes.iter().find(|n| n.is_dev_node()).copied();
        let name = match (dev_node, battery_node) {
            (Some(d), _) if !d.name.is_empty() => d.name.clone(),
            (_, Some(b)) => device::display_name(&b.name),
            _ => device::display_name(&nodes[0].name),
        };
        let kind_source = battery_node.or(dev_node).unwrap_or(nodes[0]);
        Some(DeviceState {
            id: addr,
            kind: device::classify(&kind_source.instance_id, &name),
            percent: battery_node.and_then(|n| n.battery).map(|p| p.min(100)),
            connected: dev_node.and_then(|n| n.connected).unwrap_or(false),
            name,
        })
    }
}

// ---------- WinRT part ----------

type Callback = Arc<dyn Fn(DeviceEvent) + Send + Sync>;

struct Shared {
    table: Mutex<NodeTable>,
    infos: Mutex<HashMap<String, DeviceInformation>>,
    on_event: Callback,
    /// False until the first full list is sent; single events before that are partial.
    ready: AtomicBool,
    /// (running full reads, node IDs changed while a full read runs)
    changed: Mutex<(u32, HashSet<String>)>,
}

pub struct WatcherHandle {
    _watcher: DeviceWatcher,
    shared: Arc<Shared>,
}

fn properties() -> windows_collections::IIterable<HSTRING> {
    vec![HSTRING::from(BATTERY_KEY), HSTRING::from(CONNECTED_KEY)].into()
}

fn read_node(info: &DeviceInformation) -> windows::core::Result<NodeInfo> {
    let props = info.Properties()?;
    let value = |key: &str| props.Lookup(&HSTRING::from(key)).ok().and_then(|v| v.cast::<IPropertyValue>().ok());
    Ok(NodeInfo {
        instance_id: info.Id()?.to_string(),
        name: info.Name()?.to_string(),
        battery: value(BATTERY_KEY).and_then(|v| v.GetUInt8().ok()),
        connected: value(CONNECTED_KEY).and_then(|v| v.GetBoolean().ok()),
    })
}

// Lock order: `table`, then `infos`, then `changed`. Events are sent while
// `table` is locked, so the channel order is the same as the table order.
impl Shared {
    fn emit(&self, ev: Option<DeviceEvent>) {
        if let Some(ev) = ev.filter(|_| self.ready.load(Ordering::Acquire)) {
            (self.on_event)(ev);
        }
    }

    /// Records a node that changed while a full read runs.
    fn mark_changed(&self, id: &str) {
        let mut c = self.changed.lock().unwrap();
        if c.0 > 0 {
            c.1.insert(id.to_string());
        }
    }

    fn added(&self, info: &DeviceInformation) -> windows::core::Result<()> {
        let node = read_node(info)?;
        let mut table = self.table.lock().unwrap();
        self.infos.lock().unwrap().insert(node.instance_id.clone(), info.clone());
        self.mark_changed(&node.instance_id);
        let ev = table.upsert(node);
        self.emit(ev);
        Ok(())
    }

    fn updated(&self, upd: &DeviceInformationUpdate) -> windows::core::Result<()> {
        let id = upd.Id()?.to_string();
        let mut table = self.table.lock().unwrap();
        let info = self.infos.lock().unwrap().get(&id).cloned();
        if let Some(info) = info {
            info.Update(upd)?;
            let node = read_node(&info)?;
            self.mark_changed(&id);
            let ev = table.upsert(node);
            self.emit(ev);
        }
        Ok(())
    }

    fn removed(&self, upd: &DeviceInformationUpdate) -> windows::core::Result<()> {
        let id = upd.Id()?.to_string();
        let mut table = self.table.lock().unwrap();
        self.infos.lock().unwrap().remove(&id);
        self.mark_changed(&id);
        let ev = table.remove(&id);
        self.emit(ev);
        Ok(())
    }

    /// Reads all Bluetooth nodes again and sends one `Resync`. Nodes that changed
    /// during the read keep their live state (the read result for them is older).
    fn full_read(&self) -> windows::core::Result<()> {
        self.changed.lock().unwrap().0 += 1;
        let result = self.read_and_merge();
        let mut c = self.changed.lock().unwrap();
        c.0 -= 1;
        if c.0 == 0 {
            c.1.clear();
        }
        result
    }

    fn read_and_merge(&self) -> windows::core::Result<()> {
        let list = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &HSTRING::from(AQS),
            &properties(),
            DeviceInformationKind::Device,
        )?
        .join()?;
        let mut fresh = NodeTable::default();
        let mut fresh_infos = HashMap::new();
        for info in list {
            let node = read_node(&info)?;
            fresh_infos.insert(node.instance_id.clone(), info);
            fresh.upsert(node);
        }
        let mut table = self.table.lock().unwrap();
        let mut infos = self.infos.lock().unwrap();
        let changed = self.changed.lock().unwrap().1.clone();
        for id in &changed {
            match infos.remove(id) {
                Some(live) => fresh_infos.insert(id.clone(), live),
                None => fresh_infos.remove(id),
            };
        }
        *infos = fresh_infos;
        drop(infos);
        table.merge_fresh(fresh, &changed);
        (self.on_event)(DeviceEvent::Resync(table.snapshot()));
        Ok(())
    }
}

fn spawn_full_read(shared: &Arc<Shared>) {
    let s = shared.clone();
    std::thread::spawn(move || {
        if let Err(e) = s.full_read() {
            log::error!("device read failed: {e}");
        }
    });
}

impl WatcherHandle {
    /// Reads all devices again in a background thread and sends `DeviceEvent::Resync`.
    pub fn resync(&self) {
        spawn_full_read(&self.shared);
    }
}

pub fn start(on_event: impl Fn(DeviceEvent) + Send + Sync + 'static) -> windows::core::Result<WatcherHandle> {
    let shared = Arc::new(Shared {
        table: Mutex::new(NodeTable::default()),
        infos: Mutex::new(HashMap::new()),
        on_event: Arc::new(on_event),
        ready: AtomicBool::new(false),
        changed: Mutex::new((0, HashSet::new())),
    });
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(AQS),
        &properties(),
        DeviceInformationKind::Device,
    )?;
    let s = shared.clone();
    watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, info| {
        if let Some(info) = info.as_ref() {
            if let Err(e) = s.added(info) {
                log::warn!("device added: {e}");
            }
        }
        Ok(())
    }))?;
    let s = shared.clone();
    watcher.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, upd| {
        if let Some(upd) = upd.as_ref() {
            if let Err(e) = s.updated(upd) {
                log::warn!("device updated: {e}");
            }
        }
        Ok(())
    }))?;
    let s = shared.clone();
    watcher.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, upd| {
        if let Some(upd) = upd.as_ref() {
            if let Err(e) = s.removed(upd) {
                log::warn!("device removed: {e}");
            }
        }
        Ok(())
    }))?;
    let s = shared.clone();
    watcher.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, windows::core::IInspectable>::new(
        move |_, _| {
            let table = s.table.lock().unwrap();
            s.ready.store(true, Ordering::Release);
            (s.on_event)(DeviceEvent::Resync(table.snapshot()));
            Ok(())
        },
    ))?;
    watcher.Start()?;

    let weak = Arc::downgrade(&shared);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(POLL);
            match weak.upgrade() {
                Some(s) => spawn_full_read(&s),
                None => break,
            }
        }
    });
    Ok(WatcherHandle { _watcher: watcher, shared })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{DeviceEvent, DeviceId, DeviceKind};

    const HP_PROFILE: &str = r"BTHENUM\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&1015\8&3223B969&0&5818627FF3D4_C00000000";
    const HP_DEV: &str = r"BTHENUM\DEV_5818627FF3D4\9&1&0&BLUETOOTHDEVICE_5818627FF3D4";
    const PAD: &str = r"BTHLE\DEV_408E2CAB5430\8&D3E38CA&0&408E2CAB5430";

    fn node(id: &str, name: &str, battery: Option<u8>, connected: Option<bool>) -> NodeInfo {
        NodeInfo { instance_id: id.into(), name: name.into(), battery, connected }
    }

    #[test]
    fn classic_joins_battery_and_dev_node() {
        let mut t = NodeTable::default();
        t.upsert(node(HP_PROFILE, "1000X THE COLLEXION Hands-Free AG", Some(28), Some(false)));
        let ev = t.upsert(node(HP_DEV, "1000X THE COLLEXION", None, Some(true)));
        let Some(DeviceEvent::Upsert(d)) = ev else { panic!("expected upsert, got {ev:?}") };
        assert_eq!(d.id, DeviceId(0x5818627FF3D4));
        assert_eq!(d.name, "1000X THE COLLEXION");
        assert_eq!(d.kind, DeviceKind::Headphones);
        assert_eq!(d.percent, Some(28));
        assert!(d.connected);
    }

    #[test]
    fn profile_node_connected_flag_is_ignored() {
        let mut t = NodeTable::default();
        t.upsert(node(HP_DEV, "1000X THE COLLEXION", None, Some(false)));
        let ev = t.upsert(node(HP_PROFILE, "1000X THE COLLEXION Hands-Free AG", Some(28), Some(true)));
        let Some(DeviceEvent::Upsert(d)) = ev else { panic!() };
        assert!(!d.connected);
    }

    #[test]
    fn ble_single_node() {
        let mut t = NodeTable::default();
        let ev = t.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(true)));
        let Some(DeviceEvent::Upsert(d)) = ev else { panic!() };
        assert_eq!((d.kind, d.percent, d.connected), (DeviceKind::Gamepad, Some(81), true));
    }

    #[test]
    fn non_bluetooth_node_ignored() {
        let mut t = NodeTable::default();
        assert_eq!(t.upsert(node(r"HID\VID_3434&PID_D030&MI_03\8&9&0&0000", "kbd", None, None)), None);
    }

    #[test]
    fn removing_last_node_removes_device() {
        let mut t = NodeTable::default();
        t.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(true)));
        assert_eq!(t.remove(PAD), Some(DeviceEvent::Removed(DeviceId(0x408E2CAB5430))));
    }

    #[test]
    fn removing_one_of_two_nodes_updates_device() {
        let mut t = NodeTable::default();
        t.upsert(node(HP_PROFILE, "1000X THE COLLEXION Hands-Free AG", Some(28), None));
        t.upsert(node(HP_DEV, "1000X THE COLLEXION", None, Some(true)));
        let Some(DeviceEvent::Upsert(d)) = t.remove(HP_PROFILE) else { panic!() };
        assert_eq!(d.percent, None);
    }

    fn dirty(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn merge_keeps_live_entry_changed_during_read() {
        let mut live = NodeTable::default();
        live.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(false)));
        let mut fresh = NodeTable::default();
        fresh.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(true)));
        live.merge_fresh(fresh, &dirty(&[PAD]));
        assert!(!live.snapshot()[0].connected);
    }

    #[test]
    fn merge_drops_node_removed_during_read() {
        let mut live = NodeTable::default();
        let mut fresh = NodeTable::default();
        fresh.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(true)));
        live.merge_fresh(fresh, &dirty(&[PAD]));
        assert!(live.snapshot().is_empty());
    }

    #[test]
    fn merge_takes_fresh_for_unchanged_nodes() {
        let mut live = NodeTable::default();
        live.upsert(node(PAD, "Xbox Wireless Controller", Some(50), Some(true)));
        let mut fresh = NodeTable::default();
        fresh.upsert(node(PAD, "Xbox Wireless Controller", Some(40), Some(true)));
        live.merge_fresh(fresh, &HashSet::new());
        assert_eq!(live.snapshot()[0].percent, Some(40));
    }

    #[test]
    fn snapshot_lists_each_device_once() {
        let mut t = NodeTable::default();
        t.upsert(node(HP_PROFILE, "1000X THE COLLEXION Hands-Free AG", Some(28), None));
        t.upsert(node(HP_DEV, "1000X THE COLLEXION", None, Some(true)));
        t.upsert(node(PAD, "Xbox Wireless Controller", Some(81), Some(true)));
        assert_eq!(t.snapshot().len(), 2);
    }
}
