//! App state: turns device events into tray and toast actions. No Windows code.

use crate::alerts::{AlertLevel, AlertTracker};
use crate::config::{self, Config};
use crate::device::{DeviceEvent, DeviceId, DeviceKind, DeviceState};
use crate::icon::{self, BarColor};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

/// After a reconnect, Windows still holds the battery value from before the
/// disconnect. Wait this long for a fresh value before the device shows.
pub const RECONNECT_GRACE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ShowIcon { id: DeviceId, kind: DeviceKind, percent: u8, color: BarColor, tooltip: String },
    RemoveIcon(DeviceId),
    StartBlink(DeviceId),
    StopBlink(DeviceId),
    ShowToast { id: DeviceId, name: String, percent: u8, level: AlertLevel },
}

pub struct App {
    config: Config,
    tracker: AlertTracker,
    devices: BTreeMap<DeviceId, DeviceState>,
    shown: HashMap<DeviceId, Action>,
    blinking: HashSet<DeviceId>,
    /// Reconnected devices that wait for a fresh value: start time and the old value.
    grace: HashMap<DeviceId, (Instant, Option<u8>)>,
}

fn alert_key(id: DeviceId) -> String {
    format!("{:012X}", id.0)
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            tracker: AlertTracker::new(),
            devices: BTreeMap::new(),
            shown: HashMap::new(),
            blinking: HashSet::new(),
            grace: HashMap::new(),
        }
    }

    pub fn handle(&mut self, event: DeviceEvent) -> Vec<Action> {
        self.handle_at(event, Instant::now())
    }

    pub fn handle_at(&mut self, event: DeviceEvent, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        match event {
            DeviceEvent::Upsert(d) => self.apply(d, now, &mut out),
            DeviceEvent::Removed(id) => self.remove(id, &mut out),
            DeviceEvent::Resync(list) => {
                let keep: HashSet<DeviceId> = list.iter().map(|d| d.id).collect();
                let gone: Vec<DeviceId> = self.devices.keys().filter(|id| !keep.contains(id)).copied().collect();
                for id in gone {
                    self.remove(id, &mut out);
                }
                for d in list {
                    self.apply(d, now, &mut out);
                }
            }
        }
        out
    }

    /// Shows devices whose reconnect wait has ended. Call every few seconds.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let expired: Vec<DeviceId> =
            self.grace.iter().filter(|(_, (t, _))| now >= *t + RECONNECT_GRACE).map(|(&id, _)| id).collect();
        let mut out = Vec::new();
        for id in expired {
            self.grace.remove(&id);
            if let Some(d) = self.devices.get(&id).cloned() {
                self.apply(d, now, &mut out);
            }
        }
        out
    }
    pub fn acknowledge(&mut self, id: DeviceId) -> Vec<Action> {
        if self.blinking.remove(&id) { vec![Action::StopBlink(id)] } else { Vec::new() }
    }

    pub fn reload_config(&mut self, text: &str) -> Result<Vec<Action>, String> {
        self.config = config::parse(text)?;
        let mut out = Vec::new();
        let all: Vec<DeviceState> = self.devices.values().cloned().collect();
        for d in all {
            self.apply(d, Instant::now(), &mut out);
        }
        Ok(out)
    }

    pub fn is_blinking(&self, id: DeviceId) -> bool {
        self.blinking.contains(&id)
    }

    pub fn menu_items(&self) -> Vec<String> {
        let mut items: Vec<String> = self
            .shown
            .values()
            .filter_map(|a| match a {
                Action::ShowIcon { tooltip, .. } => Some(tooltip.clone()),
                _ => None,
            })
            .collect();
        items.sort();
        items
    }

    fn apply(&mut self, d: DeviceState, now: Instant, out: &mut Vec<Action>) {
        let id = d.id;
        let was_disconnected = self.devices.get(&id).is_some_and(|p| !p.connected);
        if !d.connected {
            self.grace.remove(&id);
        } else if was_disconnected {
            self.grace.insert(id, (now, d.percent));
        }
        if let Some(&(start, old)) = self.grace.get(&id) {
            let fresh = d.percent.is_some() && d.percent != old;
            if fresh || now >= start + RECONNECT_GRACE {
                self.grace.remove(&id);
            } else {
                self.devices.insert(id, d);
                return;
            }
        }
        let visible = d.connected && !self.config.is_ignored(&d.name);
        match (visible, d.percent) {
            (true, Some(p)) => {
                let p = p.min(100);
                let icon = Action::ShowIcon {
                    id,
                    kind: d.kind,
                    percent: p,
                    color: icon::bar_color(p, &self.config.bar_colors),
                    tooltip: format!("{} — {}%", d.name, p),
                };
                if self.shown.get(&id) != Some(&icon) {
                    self.shown.insert(id, icon.clone());
                    out.push(icon);
                }
                let limits = self.config.limits_for(&d.name);
                if let Some(level) = self.tracker.update(&alert_key(id), p, &limits) {
                    out.push(Action::ShowToast { id, name: d.name.clone(), percent: p, level });
                    self.blinking.insert(id);
                    out.push(Action::StartBlink(id));
                }
            }
            _ => self.hide(id, out),
        }
        self.devices.insert(id, d);
    }

    fn remove(&mut self, id: DeviceId, out: &mut Vec<Action>) {
        self.grace.remove(&id);
        if self.devices.remove(&id).is_some() {
            // The device is gone (for example unpaired): start fresh if it comes back.
            self.tracker.forget(&alert_key(id));
            self.hide(id, out);
        }
    }

    /// Alert state stays: spec 5.1 rule 3 resets alerts only above the reset limit.
    fn hide(&mut self, id: DeviceId, out: &mut Vec<Action>) {
        if self.shown.remove(&id).is_some() {
            self.blinking.remove(&id);
            out.push(Action::RemoveIcon(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HP: DeviceId = DeviceId(1);
    const PH: DeviceId = DeviceId(2);

    fn dev(id: DeviceId, name: &str, percent: Option<u8>, connected: bool) -> DeviceState {
        DeviceState { id, name: name.into(), kind: DeviceKind::Headphones, percent, connected }
    }
    fn hp(p: Option<u8>, c: bool) -> DeviceEvent {
        DeviceEvent::Upsert(dev(HP, "1000X THE COLLEXION", p, c))
    }
    fn s26(p: u8) -> DeviceEvent {
        DeviceEvent::Upsert(dev(PH, "S26 de Luciano", Some(p), true))
    }

    #[test]
    fn connected_with_value_shows_icon() {
        let a = App::new(Config::default()).handle(hp(Some(34), true));
        assert_eq!(
            a,
            vec![Action::ShowIcon {
                id: HP,
                kind: DeviceKind::Headphones,
                percent: 34,
                color: BarColor::Yellow,
                tooltip: "1000X THE COLLEXION — 34%".into(),
            }]
        );
    }

    #[test]
    fn connected_without_value_has_no_icon() {
        assert!(App::new(Config::default()).handle(hp(None, true)).is_empty());
    }

    #[test]
    fn disconnected_has_no_icon() {
        assert!(App::new(Config::default()).handle(hp(Some(90), false)).is_empty());
    }

    #[test]
    fn disconnect_removes_icon() {
        let mut app = App::new(Config::default());
        app.handle(hp(Some(90), true));
        assert_eq!(app.handle(hp(Some(90), false)), vec![Action::RemoveIcon(HP)]);
    }

    #[test]
    fn low_value_toasts_and_blinks() {
        let a = App::new(Config::default()).handle(hp(Some(20), true));
        assert_eq!(a.len(), 3);
        assert!(matches!(a[0], Action::ShowIcon { percent: 20, color: BarColor::Red, .. }));
        assert_eq!(
            a[1],
            Action::ShowToast { id: HP, name: "1000X THE COLLEXION".into(), percent: 20, level: AlertLevel::Low }
        );
        assert_eq!(a[2], Action::StartBlink(HP));
    }

    #[test]
    fn acknowledge_stops_blink_once() {
        let mut app = App::new(Config::default());
        app.handle(hp(Some(20), true));
        assert_eq!(app.acknowledge(HP), vec![Action::StopBlink(HP)]);
        assert!(app.acknowledge(HP).is_empty());
    }

    #[test]
    fn ignored_device_hidden() {
        let cfg = config::parse("ignore = [\"S26 de Luciano\"]").unwrap();
        assert!(App::new(cfg).handle(s26(20)).is_empty());
    }

    #[test]
    fn resync_removes_missing() {
        let mut app = App::new(Config::default());
        app.handle(hp(Some(90), true));
        app.handle(s26(80));
        let a = app.handle(DeviceEvent::Resync(vec![dev(HP, "1000X THE COLLEXION", Some(90), true)]));
        assert_eq!(a, vec![Action::RemoveIcon(PH)]);
    }

    #[test]
    fn reload_keeps_previous_on_error() {
        let mut app = App::new(Config::default());
        assert!(app.reload_config("[limits").is_err());
        let a = app.handle(s26(20));
        assert!(a.iter().any(|x| matches!(x, Action::ShowToast { level: AlertLevel::Low, .. })));
    }

    #[test]
    fn reload_hides_newly_ignored() {
        let mut app = App::new(Config::default());
        app.handle(s26(80));
        let a = app.reload_config("ignore = [\"S26 de Luciano\"]").unwrap();
        assert_eq!(a, vec![Action::RemoveIcon(PH)]);
    }

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + std::time::Duration::from_secs(secs)
    }

    #[test]
    fn reconnect_at_same_low_level_no_second_toast() {
        let t0 = Instant::now();
        let mut app = App::new(Config::default());
        app.handle_at(hp(Some(15), true), t0);
        app.handle_at(hp(Some(15), false), at(t0, 1));
        app.handle_at(hp(Some(15), true), at(t0, 2));
        let a = app.tick(at(t0, 30));
        assert!(a.iter().any(|x| matches!(x, Action::ShowIcon { percent: 15, .. })));
        assert!(!a.iter().any(|x| matches!(x, Action::ShowToast { .. })));
    }

    #[test]
    fn reconnect_waits_for_fresh_value() {
        let t0 = Instant::now();
        let mut app = App::new(Config::default());
        app.handle_at(hp(Some(90), true), t0);
        app.handle_at(hp(Some(18), false), at(t0, 1));
        assert!(app.handle_at(hp(Some(18), true), at(t0, 2)).is_empty());
        let a = app.handle_at(hp(Some(100), true), at(t0, 5));
        assert!(matches!(a.as_slice(), [Action::ShowIcon { percent: 100, .. }]));
    }

    #[test]
    fn reconnect_grace_expires() {
        let t0 = Instant::now();
        let mut app = App::new(Config::default());
        app.handle_at(hp(Some(90), true), t0);
        app.handle_at(hp(Some(18), false), at(t0, 1));
        app.handle_at(hp(Some(18), true), at(t0, 2));
        assert!(app.tick(at(t0, 10)).is_empty());
        let a = app.tick(at(t0, 2 + RECONNECT_GRACE.as_secs()));
        assert!(a.iter().any(|x| matches!(x, Action::ShowIcon { percent: 18, .. })));
        assert!(a.iter().any(|x| matches!(x, Action::ShowToast { level: AlertLevel::Low, .. })));
    }

    #[test]
    fn same_name_devices_alert_separately() {
        let mut app = App::new(Config::default());
        app.handle(DeviceEvent::Upsert(dev(HP, "Pad", Some(15), true)));
        let a = app.handle(DeviceEvent::Upsert(dev(PH, "Pad", Some(15), true)));
        assert!(a.iter().any(|x| matches!(x, Action::ShowToast { .. })));
    }

    #[test]
    fn menu_items_sorted() {
        let mut app = App::new(Config::default());
        app.handle(s26(80));
        app.handle(hp(Some(34), true));
        assert_eq!(app.menu_items(), vec!["1000X THE COLLEXION — 34%", "S26 de Luciano — 80%"]);
    }
}
