//! What an app leaves in the home directory: config, data and cache folders matched to it.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::distro;
use crate::model::{App, AppFootprint, AppKind, Category, Confidence, Residue};
use crate::run;

/// Folder names that belong to the desktop or to many apps at once. Never offered as residue.
pub const SHARED_NAMES: &[&str] = &[
    "applications",
    "autostart",
    "dconf",
    "desktop-directories",
    "environment.d",
    "fontconfig",
    "fonts",
    "gnome-session",
    "gtk-2.0",
    "gtk-3.0",
    "gtk-4.0",
    "gvfs-metadata",
    "ibus",
    "icons",
    "keyrings",
    "menus",
    "mime",
    "nautilus",
    "pulse",
    "pipewire",
    "wireplumber",
    "recently-used.xbel",
    "sounds",
    "systemd",
    "themes",
    "trash",
    "user-dirs.dirs",
    "user-dirs.locale",
    "xdg-desktop-portal",
    "flatpak",
    "snap",
    "kdeconnect",
    "kwalletd",
    "gnupg",
    "ssh",
    "evolution",
    "goa-1.0",
    "gnome-shell",
    "mimeapps.list",
    "monitors.xml",
    "session",
    "sessions",
    "tracker",
    "tracker3",
    "thumbnails",
    "mesa_shader_cache",
    "mesa_shader_cache_db",
    "fontconfig",
    "appnux",
];

/// Folder names that do not match the package or app name.
const ALIASES: &[(&str, &[&str])] = &[
    ("firefox", &[".mozilla", "mozilla"]),
    ("thunderbird", &[".thunderbird", "thunderbird"]),
    ("code", &["Code", ".vscode"]),
    ("google-chrome-stable", &["google-chrome"]),
    ("brave-browser", &["BraveSoftware"]),
    ("chromium-browser", &["chromium"]),
    ("steam", &[".steam", "Steam"]),
    ("telegram-desktop", &["TelegramDesktop"]),
    ("obs-studio", &["obs-studio"]),
    ("gimp", &["GIMP"]),
    ("slack", &["Slack"]),
    ("zoom", &[".zoom"]),
];

const GENERIC: &[&str] = &[
    "env", "sh", "bash", "python", "python3", "java", "electron", "snap", "flatpak", "appimage",
    "config", "local", "cache", "share", "var", "app", "apps", "apprun", "run", "bin", "usr",
    "gtk", "qt", "desktop", "launcher", "start",
];

struct Root {
    path: PathBuf,
    category: Category,
}

fn roots(home: &Path) -> Vec<Root> {
    vec![
        Root {
            path: home.join(".config"),
            category: Category::Config,
        },
        Root {
            path: home.join(".local/share"),
            category: Category::Data,
        },
        Root {
            path: home.join(".local/state"),
            category: Category::Data,
        },
        Root {
            path: home.join(".cache"),
            category: Category::Cache,
        },
    ]
}

/// Lowercase name → entries, for each root and the home directory's dot folders.
struct Index {
    by_name: HashMap<String, Vec<(PathBuf, Category)>>,
}

impl Index {
    fn build(home: &Path) -> Self {
        let mut by_name: HashMap<String, Vec<(PathBuf, Category)>> = HashMap::new();
        for root in roots(home) {
            let Ok(entries) = fs::read_dir(&root.path) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                by_name
                    .entry(name)
                    .or_default()
                    .push((entry.path(), root.category));
            }
        }
        if let Ok(entries) = fs::read_dir(home) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with('.') && name.len() > 2 {
                    by_name
                        .entry(name)
                        .or_default()
                        .push((entry.path(), Category::Data));
                }
            }
        }
        Index { by_name }
    }
}

fn usable(id: &str) -> bool {
    let lower = id.to_lowercase();
    lower.len() >= 3
        && !GENERIC.contains(&lower.as_str())
        && !SHARED_NAMES.contains(&lower.as_str())
}

/// Candidate folders for one app, before sizing.
fn matches(app: &App, index: &Index, home: &Path) -> Vec<(PathBuf, Category, Confidence)> {
    let mut found: Vec<(PathBuf, Category, Confidence)> = Vec::new();
    match app.kind {
        AppKind::Flatpak => {
            let base = home.join(".var/app").join(&app.package);
            for (sub, category) in [
                ("config", Category::Config),
                ("data", Category::Data),
                ("cache", Category::Cache),
            ] {
                let path = base.join(sub);
                if path.exists() {
                    found.push((path, category, Confidence::High));
                }
            }
        }
        AppKind::Snap => {
            let path = home.join("snap").join(&app.package);
            if path.exists() {
                found.push((path, Category::Data, Confidence::High));
            }
        }
        _ => {}
    }
    let primary = app.package.to_lowercase();
    let mut names: Vec<(String, Confidence)> = app
        .identifiers
        .iter()
        .filter(|id| usable(id))
        .map(|id| {
            let lower = id.to_lowercase();
            let confidence = if lower == primary {
                Confidence::High
            } else {
                Confidence::Medium
            };
            (lower, confidence)
        })
        .collect();
    for (pkg, aliases) in ALIASES {
        if app.package == *pkg || app.identifiers.iter().any(|i| i == pkg) {
            names.extend(aliases.iter().map(|a| (a.to_lowercase(), Confidence::High)));
        }
    }
    for (name, confidence) in names {
        for key in [name.clone(), format!(".{name}")] {
            let Some(hits) = index.by_name.get(&key) else {
                continue;
            };
            for (path, category) in hits {
                // A dotted name only counts in the home directory itself.
                if key.starts_with('.') && path.parent() != Some(home) && !name.starts_with('.') {
                    continue;
                }
                if let Some(existing) = found.iter_mut().find(|(p, _, _)| p == path) {
                    existing.2 = existing.2.max(confidence);
                } else {
                    found.push((path.clone(), *category, confidence));
                }
            }
        }
    }
    found
}

/// Footprints of every app. A folder that matches several apps drops to low confidence: it
/// may still be needed by the others.
pub fn footprints(apps: &[App]) -> Vec<AppFootprint> {
    let home = distro::home();
    let index = Index::build(&home);
    let candidates: Vec<Vec<(PathBuf, Category, Confidence)>> =
        apps.iter().map(|app| matches(app, &index, &home)).collect();

    let mut owners: HashMap<&PathBuf, usize> = HashMap::new();
    for list in &candidates {
        for (path, _, _) in list {
            *owners.entry(path).or_default() += 1;
        }
    }
    let unique: HashSet<&PathBuf> = candidates.iter().flatten().map(|(p, _, _)| p).collect();
    let sizes: HashMap<PathBuf, u64> = unique
        .into_par_iter()
        .map(|p| (p.clone(), run::disk_usage(p)))
        .collect();

    apps.iter()
        .zip(&candidates)
        .map(|(app, list)| {
            let residue: Vec<Residue> = list
                .iter()
                .map(|(path, category, confidence)| Residue {
                    path: path.display().to_string(),
                    size: sizes.get(path).copied().unwrap_or(0),
                    category: *category,
                    confidence: if owners.get(path).copied().unwrap_or(0) > 1 {
                        Confidence::Low
                    } else {
                        *confidence
                    },
                })
                .collect();
            let sum = |c: Category| {
                residue
                    .iter()
                    .filter(|r| r.category == c)
                    .map(|r| r.size)
                    .sum()
            };
            let last_activity = list
                .iter()
                .filter(|(_, category, confidence)| {
                    *category != Category::Cache && *confidence >= Confidence::Medium
                })
                .filter_map(|(path, _, _)| newest_change(path))
                .max();
            AppFootprint {
                last_activity,
                id: app.id.clone(),
                config: sum(Category::Config),
                data: sum(Category::Data),
                cache: sum(Category::Cache),
                residue,
            }
        })
        .collect()
}

/// Every lowercase identifier of the installed apps, for the leftover scanner.
pub fn known_identifiers(apps: &[App]) -> HashSet<String> {
    let mut set: HashSet<String> = apps
        .iter()
        .flat_map(|a| a.identifiers.iter().map(|i| i.to_lowercase()))
        .collect();
    // First word of each name too ("antigravity" for antigravity-ide): a leftover scan that
    // keeps too much is safer than one that offers a live app's folder.
    let stems: Vec<String> = set
        .iter()
        .filter_map(|i| {
            i.split(['-', '_', '.'])
                .next()
                .filter(|s| s.len() >= 4)
                .map(str::to_owned)
        })
        .collect();
    set.extend(stems);
    for (pkg, aliases) in ALIASES {
        if set.contains(*pkg) {
            set.extend(
                aliases
                    .iter()
                    .map(|a| a.trim_start_matches('.').to_lowercase()),
            );
        }
    }
    set
}

/// Newest modification time of a folder and its direct children, Unix seconds. Shallow on
/// purpose: an app touches its own config or database file when it runs.
fn newest_change(path: &Path) -> Option<u64> {
    let stamp = |meta: fs::Metadata| {
        meta.modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    };
    let own = fs::metadata(path).ok().and_then(stamp);
    let children = fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok().and_then(stamp))
        .max();
    own.max(children)
}
