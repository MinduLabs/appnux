//! Settings kept between runs, in `$XDG_CONFIG_HOME/appnux/settings.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// "system", "light" or "dark".
    pub theme: String,
    pub journal_keep_mb: u64,
    /// OpenGL rendering: smoother, about twice the memory of the cairo renderer.
    pub hardware_acceleration: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: "system".into(),
            journal_keep_mb: 100,
            hardware_acceleration: false,
        }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| appnux_core::distro::home().join(".config"));
    base.join("appnux/settings.json")
}

pub fn load() -> Prefs {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(prefs: &Prefs) {
    let path = path();
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| serde_json::to_string_pretty(prefs).map_err(std::io::Error::other))
        .and_then(|text| fs::write(&path, text));
    if let Err(err) = written {
        eprintln!("appnux: settings not saved: {err}");
    }
}
