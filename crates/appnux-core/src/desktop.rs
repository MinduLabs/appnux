//! Desktop entries (`.desktop` files) and icon lookup.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;

use crate::distro;

#[derive(Debug, Clone)]
pub struct DesktopEntry {
    /// File name without `.desktop`: the desktop file ID.
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub comment: String,
    pub exec: String,
    pub icon: String,
    pub wm_class: String,
    pub flatpak: Option<String>,
    pub snap: Option<String>,
}

/// Directories holding `applications/`, highest priority first (the user's own entries shadow
/// the system's, as the spec says).
pub fn data_dirs() -> Vec<PathBuf> {
    let home = distro::home();
    let mut dirs = vec![
        std::env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), Into::into),
    ];
    let system = crate::run::session_data_dirs();
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    for extra in [
        home.join(".local/share/flatpak/exports/share"),
        "/var/lib/flatpak/exports/share".into(),
        "/var/lib/snapd/desktop".into(),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

/// Every visible application entry, one per desktop file ID.
pub fn entries() -> Vec<DesktopEntry> {
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for dir in data_dirs() {
        let Ok(files) = fs::read_dir(dir.join("applications")) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            if seen.insert(id.clone(), ()).is_some() {
                continue;
            }
            if let Some(entry) = parse(&path, id) {
                out.push(entry);
            }
        }
    }
    out
}

fn parse(path: &Path, id: String) -> Option<DesktopEntry> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_main = false;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.entry(key.trim()).or_insert(value.trim());
        }
    }
    let get = |key: &str| fields.get(key).copied().unwrap_or("").to_owned();
    if get("Type") != "Application" || get("NoDisplay") == "true" || get("Hidden") == "true" {
        return None;
    }
    let name = get("Name");
    if name.is_empty() {
        return None;
    }
    let flatpak = fields.get("X-Flatpak").map(|s| (*s).to_owned());
    let snap = fields
        .get("X-SnapInstanceName")
        .map(|s| (*s).to_owned())
        .or_else(|| {
            path.starts_with("/var/lib/snapd/desktop")
                .then(|| id.split('_').next().unwrap_or(&id).to_owned())
        });
    Some(DesktopEntry {
        id,
        path: path.to_owned(),
        name,
        comment: get("Comment"),
        exec: get("Exec"),
        icon: get("Icon"),
        wm_class: get("StartupWMClass"),
        flatpak,
        snap,
    })
}

/// The program an `Exec` line runs, without `env`, `VAR=value` and field codes.
pub fn exec_program(exec: &str) -> Option<String> {
    split_exec(exec)
        .into_iter()
        .find(|token| token != "env" && !token.contains('='))
}

/// Splits an `Exec` value on spaces, keeping double-quoted arguments whole.
fn split_exec(exec: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in exec.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Resolves an `Icon=` value to a `data:` URL: an absolute path, or a name looked up in the
/// hicolor theme, the Flatpak and snap exports and pixmaps. Prefers SVG, then the largest PNG.
pub fn icon_data_url(icon: &str, index: &IconIndex) -> Option<String> {
    if icon.is_empty() {
        return None;
    }
    let path = if icon.starts_with('/') {
        Some(PathBuf::from(icon))
    } else {
        index.find(icon)
    }?;
    let bytes = fs::read(&path).ok()?;
    if bytes.len() > 160 * 1024 {
        return None;
    }
    let mime = match path.extension()?.to_str()? {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "xpm" => return None,
        _ => "image/png",
    };
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{data}"))
}

/// Icon name → best file, built once per scan.
pub struct IconIndex {
    best: HashMap<String, (u32, PathBuf)>,
}

impl IconIndex {
    pub fn build() -> Self {
        let mut index = IconIndex {
            best: HashMap::new(),
        };
        let home = distro::home();
        let mut roots: Vec<PathBuf> = data_dirs().into_iter().map(|d| d.join("icons")).collect();
        roots.push(home.join(".icons"));
        for root in roots {
            for theme in ["hicolor", "Adwaita", "breeze", "Papirus", "Yaru"] {
                index.scan_theme(&root.join(theme));
            }
        }
        for dir in data_dirs() {
            index.scan_flat(&dir.join("pixmaps"), 1);
        }
        index.scan_flat(Path::new("/usr/share/pixmaps"), 1);
        index
    }

    fn scan_theme(&mut self, theme: &Path) {
        let Ok(sizes) = fs::read_dir(theme) else {
            return;
        };
        for size_dir in sizes.flatten() {
            let label = size_dir.file_name().to_string_lossy().into_owned();
            // Closest to 64px wins: a 512px PNG decodes to 1 MB of pixels for a 26px row.
            let rank = if label == "scalable" {
                500
            } else {
                label
                    .split('x')
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .map_or(0, |n| {
                        if n >= 48 {
                            1000 - n.abs_diff(64).min(900)
                        } else {
                            n
                        }
                    })
            };
            self.scan_flat(&size_dir.path().join("apps"), rank);
        }
    }

    fn scan_flat(&mut self, dir: &Path, rank: u32) {
        let Ok(files) = fs::read_dir(dir) else {
            return;
        };
        for file in files.flatten() {
            let path = file.path();
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let ext_rank = match path.extension().and_then(|e| e.to_str()) {
                Some("svg" | "png") => rank,
                _ => continue,
            };
            let better = self.best.get(&stem).is_none_or(|(r, _)| ext_rank > *r);
            if better {
                self.best.insert(stem, (ext_rank, path));
            }
        }
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim_end_matches(".png").trim_end_matches(".svg");
        self.best.get(name).map(|(_, p)| p.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_program_skips_env() {
        assert_eq!(
            exec_program("env BAMF_DESKTOP_FILE_HINT=/x.desktop /snap/bin/firefox %u").as_deref(),
            Some("/snap/bin/firefox")
        );
        assert_eq!(
            exec_program("\"/opt/My App/app\" %F").as_deref(),
            Some("/opt/My App/app")
        );
        assert_eq!(
            exec_program("vlc --started-from-file %U").as_deref(),
            Some("vlc")
        );
    }
}
