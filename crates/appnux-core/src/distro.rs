//! Which distro family and package backend this machine uses.

use std::fs;

use crate::model::SystemInfo;
use crate::run;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Apt,
    Pacman,
    Dnf,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Apt => "apt",
            Backend::Pacman => "pacman",
            Backend::Dnf => "dnf",
        }
    }
}

pub struct OsRelease {
    pub pretty: String,
    pub id: String,
    pub id_like: String,
}

pub fn os_release() -> OsRelease {
    let text = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let get = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    OsRelease {
        pretty: get("PRETTY_NAME"),
        id: get("ID"),
        id_like: get("ID_LIKE"),
    }
}

/// From `ID` / `ID_LIKE` first, then from the tools present (a derivative may say neither).
pub fn backend() -> Option<Backend> {
    let os = os_release();
    let ids = format!("{} {}", os.id, os.id_like);
    let has = |word: &str| ids.split_whitespace().any(|id| id == word);
    if has("debian") || has("ubuntu") {
        return Some(Backend::Apt);
    }
    if has("arch") {
        return Some(Backend::Pacman);
    }
    if has("fedora") || has("rhel") || has("centos") {
        return Some(Backend::Dnf);
    }
    if run::has_program("dpkg-query") {
        Some(Backend::Apt)
    } else if run::has_program("pacman") {
        Some(Backend::Pacman)
    } else if run::has_program("rpm") {
        Some(Backend::Dnf)
    } else {
        None
    }
}

pub fn family(backend: Option<Backend>) -> &'static str {
    match backend {
        Some(Backend::Apt) => "Debian / Ubuntu",
        Some(Backend::Pacman) => "Arch",
        Some(Backend::Dnf) => "Fedora / RHEL",
        None => "Other",
    }
}

pub fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map_or_else(|| "/".into(), Into::into)
}

pub fn system_info() -> SystemInfo {
    let os = os_release();
    let backend = backend();
    SystemInfo {
        distro: if os.pretty.is_empty() {
            "Linux".into()
        } else {
            os.pretty
        },
        family: family(backend).into(),
        backend: backend.map(|b| b.name().to_owned()),
        desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Unknown".into()),
        home: home().display().to_string(),
        has_flatpak: run::has_program("flatpak"),
        has_snap: run::has_program("snap"),
        has_docker: run::has_program("docker"),
        kernel: fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_owned())
            .unwrap_or_default(),
    }
}
