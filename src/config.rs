//! Settings file (`config.toml`): types, defaults, parse, validate, load, save.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub low: u8,
    pub critical: u8,
    pub empty: u8,
    pub reset: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self { low: 20, critical: 10, empty: 5, reset: 25 }
    }
}

impl Limits {
    fn validate(&self) -> Result<(), String> {
        if self.empty < self.critical && self.critical < self.low && self.low < self.reset && self.reset <= 100 {
            Ok(())
        } else {
            Err(format!(
                "limits must follow empty < critical < low < reset <= 100 (got {}/{}/{}/{})",
                self.empty, self.critical, self.low, self.reset
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarColors {
    pub green_above: u8,
    pub red_at_or_below: u8,
}

impl Default for BarColors {
    fn default() -> Self {
        Self { green_above: 50, red_at_or_below: 20 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceOverride {
    pub limits: Option<Limits>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub start_with_windows: bool,
    pub ignore: Vec<String>,
    pub limits: Limits,
    pub bar_colors: BarColors,
    pub devices: HashMap<String, DeviceOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            start_with_windows: true,
            ignore: Vec::new(),
            limits: Limits::default(),
            bar_colors: BarColors::default(),
            devices: HashMap::new(),
        }
    }
}

impl Config {
    pub fn limits_for(&self, device_name: &str) -> Limits {
        self.devices
            .get(device_name)
            .and_then(|d| d.limits)
            .unwrap_or(self.limits)
    }

    pub fn is_ignored(&self, device_name: &str) -> bool {
        self.ignore.iter().any(|n| n == device_name)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.limits.validate()?;
        for (name, d) in &self.devices {
            if let Some(l) = d.limits {
                l.validate().map_err(|e| format!("device \"{name}\": {e}"))?;
            }
        }
        let b = self.bar_colors;
        if b.red_at_or_below >= b.green_above || b.green_above > 100 {
            return Err("bar_colors must follow red_at_or_below < green_above <= 100".into());
        }
        Ok(())
    }
}

pub fn parse(text: &str) -> Result<Config, String> {
    let cfg: Config = toml::from_str(text).map_err(|e| e.to_string())?;
    cfg.validate()?;
    Ok(cfg)
}

/// Changes only the `start_with_windows` line, so comments and other settings stay.
/// Refuses an invalid file, so the app never writes over the user's text.
pub fn set_start_flag(text: &str, on: bool) -> Result<String, String> {
    parse(text)?;
    let new_line = format!("start_with_windows = {on}");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if !found && l.trim_start().starts_with("start_with_windows") {
                found = true;
                new_line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.insert(0, new_line);
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') || !found {
        result.push('\n');
    }
    parse(&result)?;
    Ok(result)
}

pub fn load_or_create(path: &Path) -> Result<(Config, bool), String> {
    if path.exists() {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        return parse(&text).map(|c| (c, false));
    }
    let cfg = Config::default();
    save(path, &cfg)?;
    Ok((cfg, true))
}

pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("battery-status").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let c = parse("").unwrap();
        assert_eq!((c.limits.low, c.limits.critical, c.limits.empty, c.limits.reset), (20, 10, 5, 25));
        assert_eq!((c.bar_colors.green_above, c.bar_colors.red_at_or_below), (50, 20));
        assert!(c.start_with_windows);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let c = parse("[limits]\nlow = 22").unwrap();
        assert_eq!(c.limits.low, 22);
        assert_eq!(c.limits.critical, 10);
    }

    #[test]
    fn invalid_limits_rejected() {
        assert!(parse("[limits]\nlow = 10\ncritical = 20").is_err());
    }

    #[test]
    fn bad_toml_rejected() {
        assert!(parse("[limits").is_err());
    }

    #[test]
    fn override_used_for_named_device() {
        let c = parse(
            "[devices.\"S26 de Luciano\".limits]\nlow = 30\ncritical = 15\nempty = 5\nreset = 35",
        )
        .unwrap();
        assert_eq!(c.limits_for("S26 de Luciano").low, 30);
        assert_eq!(c.limits_for("Other").low, 20);
    }

    #[test]
    fn ignore_matches_exact_name() {
        let c = parse("ignore = [\"WF-1000XM5\"]").unwrap();
        assert!(c.is_ignored("WF-1000XM5"));
        assert!(!c.is_ignored("WH-1000XM6"));
    }

    #[test]
    fn start_flag_keeps_other_lines() {
        let text = "# my note\nstart_with_windows = true\nignore = [\"X\"]\n";
        let out = set_start_flag(text, false).unwrap();
        assert_eq!(out, "# my note\nstart_with_windows = false\nignore = [\"X\"]\n");
    }

    #[test]
    fn start_flag_added_when_missing() {
        let out = set_start_flag("ignore = []\n", false).unwrap();
        assert!(!parse(&out).unwrap().start_with_windows);
        assert!(out.contains("ignore = []"));
    }

    #[test]
    fn start_flag_refuses_invalid_file() {
        assert!(set_start_flag("[limits", true).is_err());
    }

    #[test]
    fn load_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let (c, created) = load_or_create(&path).unwrap();
        assert!(created);
        assert_eq!(c, Config::default());
        assert!(path.exists());
        let (_, created_again) = load_or_create(&path).unwrap();
        assert!(!created_again);
    }
}
