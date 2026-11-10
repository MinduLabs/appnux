//! The cleaner: caches and temporary data only. Nothing here may get in the way of work.
//!
//! What is in scope, and why (sources in docs/CLEANER.md):
//! - Package manager download caches: the packages are installed already.
//! - The system journal above a size the user picks.
//! - Thumbnails: file managers make them again.
//! - Developer tool caches, through each tool's own command where it has one: they are
//!   rebuilt or downloaded again when needed.
//! - Caches of apps that are not running.
//!
//! Out of scope on purpose: project folders (node_modules, Cargo target), Docker images,
//! containers and volumes, and folders no app claims. Removing them changes how a project or a
//! service behaves, which is not cleaning.
//!
//! Data of removed apps and the Trash are listed for the user to decide; never ticked.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::actions::{Op, RootOp};
use crate::apps;
use crate::distro::{self, Backend};
use crate::model::{App, Confidence};
use crate::run;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanItem {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub detail: String,
    /// Bytes freed. `None` when only the tool knows (pnpm frees unreferenced packages only).
    pub size: Option<u64>,
    pub confidence: Confidence,
    pub needs_root: bool,
    /// Ticked by default: only items that are safe to remove now.
    pub recommended: bool,
    /// What cleaning runs, shown in the preview.
    pub action: String,
    /// What cleaning does, exactly; `actions::execute` runs it.
    pub op: Op,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanGroup {
    pub id: String,
    pub title: String,
    /// One line: what it is and what happens after cleaning.
    pub description: String,
    pub items: Vec<CleanItem>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub journal_keep_mb: u64,
}

fn display(path: &Path, home: &Path) -> String {
    path.strip_prefix(home).map_or_else(
        |_| path.display().to_string(),
        |p| format!("~/{}", p.display()),
    )
}

struct Spec<'a> {
    id: &'a str,
    title: &'a str,
    path: PathBuf,
    detail: &'a str,
    action: String,
    recommended: bool,
}

/// "Delete folder", or a tool's own command line (fixed strings from this file).
fn folder_op(action: &str, path: &Path) -> Op {
    if action == "Delete folder" {
        return Op::Delete {
            path: path.to_owned(),
        };
    }
    let mut words = action.split_whitespace().map(str::to_owned);
    let program = words.next().unwrap_or_default();
    Op::Tool {
        program,
        args: words.collect(),
    }
}

fn folder_item(spec: Spec<'_>, home: &Path, size_known: bool) -> Option<CleanItem> {
    if !spec.path.is_dir() {
        return None;
    }
    let size = run::disk_usage(&spec.path);
    if size < 1024 * 1024 {
        return None;
    }
    Some(CleanItem {
        id: spec.id.into(),
        title: spec.title.into(),
        path: Some(display(&spec.path, home)),
        detail: spec.detail.into(),
        size: size_known.then_some(size),
        confidence: Confidence::High,
        needs_root: false,
        recommended: spec.recommended,
        op: folder_op(&spec.action, &spec.path),
        action: spec.action,
    })
}

/// Every section. Sections are fixed; their items depend on what this machine has.
pub fn scan(installed: &[App], options: ScanOptions) -> Vec<CleanGroup> {
    let home = distro::home();
    let (system, (developer, (app_caches, (removed, trash)))) = rayon::join(
        || system_caches(options),
        || {
            rayon::join(
                || developer_caches(&home),
                || {
                    rayon::join(
                        || app_caches(&home, installed),
                        || rayon::join(|| removed_apps(&home), || trash(&home)),
                    )
                },
            )
        },
    );
    let mut groups = vec![
        CleanGroup {
            id: "system".into(),
            title: "System caches".into(),
            description: "Downloaded packages that are installed already, and old system logs. Asks for your password.".into(),
            items: system,
        },
        CleanGroup {
            id: "thumbnails".into(),
            title: "Thumbnails".into(),
            description: "Previews made by the file manager. Made again when you open a folder.".into(),
            items: thumbnails(&home).into_iter().collect(),
        },
        CleanGroup {
            id: "developer".into(),
            title: "Developer tool caches".into(),
            description: "Downloads kept by npm, Yarn, pnpm, pip, uv, Go and Cargo. Your projects are not touched; tools fetch again when needed.".into(),
            items: developer,
        },
        CleanGroup {
            id: "apps".into(),
            title: "App caches".into(),
            description: "Caches of apps that are not running. The app rebuilds them; the first start may be slower.".into(),
            items: app_caches,
        },
        CleanGroup {
            id: "shortcuts".into(),
            title: "Broken shortcuts".into(),
            description: "Entries in your app menu for programs that were deleted. They open nothing.".into(),
            items: dead_shortcuts(),
        },
        CleanGroup {
            id: "snaps".into(),
            title: "Old snap revisions".into(),
            description: "The previous version snap keeps so you can roll back. Removing it drops that option.".into(),
            items: old_snaps(),
        },
        CleanGroup {
            id: "removed".into(),
            title: "Data of removed apps".into(),
            description: "Settings and files of Flatpaks and snaps that are no longer installed. Deleted for good, so never ticked.".into(),
            items: removed,
        },
        CleanGroup {
            id: "trash".into(),
            title: "Trash".into(),
            description: "Files you deleted. Emptying it cannot be undone.".into(),
            items: trash.into_iter().collect(),
        },
    ];
    groups.retain(|g| !g.items.is_empty());
    groups
}

// ---------- System ----------

fn system_caches(options: ScanOptions) -> Vec<CleanItem> {
    let mut items = Vec::new();
    let package_cache = |dir: &str, title: &str, action: &str| {
        let size = run::disk_usage(Path::new(dir));
        (size > 1024 * 1024).then(|| CleanItem {
            id: format!("pkgcache:{dir}"),
            title: title.into(),
            path: Some(dir.into()),
            detail: "Installed already; downloaded again only for a reinstall".into(),
            size: Some(size),
            confidence: Confidence::High,
            needs_root: true,
            recommended: true,
            action: action.into(),
            op: Op::Root(RootOp::PackageCacheClean),
        })
    };
    match distro::backend() {
        Some(Backend::Apt) => items.extend(package_cache(
            "/var/cache/apt/archives",
            "APT package cache",
            "apt-get clean",
        )),
        Some(Backend::Pacman) => {
            // paccache keeps the newest version of each package, so a downgrade still works.
            let action = if run::has_program("paccache") {
                "paccache -rk1"
            } else {
                "pacman -Sc"
            };
            items.extend(package_cache(
                "/var/cache/pacman/pkg",
                "pacman package cache",
                action,
            ));
        }
        Some(Backend::Dnf) => {
            for dir in ["/var/cache/libdnf5", "/var/cache/dnf"] {
                items.extend(package_cache(
                    dir,
                    "DNF package cache",
                    "dnf clean packages",
                ));
            }
        }
        None => {}
    }
    items.extend(journal(options.journal_keep_mb));
    items
}

fn journal(keep_mb: u64) -> Option<CleanItem> {
    let out = run::output("journalctl", &["--disk-usage"])?;
    let usage = out
        .split("take up ")
        .nth(1)?
        .split_whitespace()
        .next()
        .and_then(run::parse_human_size)?;
    let reclaim = usage.saturating_sub(keep_mb * 1024 * 1024);
    (reclaim > 1024 * 1024).then(|| CleanItem {
        id: "journal".into(),
        title: "System logs (systemd journal)".into(),
        path: Some("/var/log/journal".into()),
        detail: format!("Logs of system services; keeps the newest {keep_mb} MB"),
        size: Some(reclaim),
        confidence: Confidence::High,
        needs_root: true,
        recommended: true,
        action: format!("journalctl --vacuum-size={keep_mb}M"),
        op: Op::Root(RootOp::JournalVacuum { keep_mb }),
    })
}

fn old_snaps() -> Vec<CleanItem> {
    apps::snap_list()
        .into_iter()
        .filter(|(_, info)| info.disabled)
        .map(|(name, info)| {
            let file = apps::snap_file(&name, &info.revision);
            CleanItem {
                id: format!("snap:{name}:{}", info.revision),
                title: format!("{name} {}", info.version),
                path: Some(file.display().to_string()),
                detail: format!("Revision {}, not in use", info.revision),
                size: Some(fs::metadata(&file).map(|m| m.len()).unwrap_or(0)),
                confidence: Confidence::High,
                needs_root: true,
                recommended: false,
                action: format!("snap remove {name} --revision={}", info.revision),
                op: Op::Root(RootOp::SnapRemoveRevision {
                    name: name.clone(),
                    revision: info.revision.clone(),
                }),
            }
        })
        .collect()
}

// ---------- Your account ----------

fn thumbnails(home: &Path) -> Option<CleanItem> {
    folder_item(
        Spec {
            id: "thumbnails",
            title: "Thumbnail cache",
            path: home.join(".cache/thumbnails"),
            detail: "Made again when needed",
            action: "Delete folder".into(),
            recommended: true,
        },
        home,
        true,
    )
}

/// Developer caches, cleaned with the tool's own command when it is installed: the tool knows
/// its layout and what is safe. A folder is deleted only where the tool has no command.
fn developer_caches(home: &Path) -> Vec<CleanItem> {
    let tool = |program: &str, command: &str| {
        if run::has_program(program) {
            command.to_owned()
        } else {
            "Delete folder".to_owned()
        }
    };
    let go_cache = run::output("go", &["env", "GOCACHE"])
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".cache/go-build"));
    let specs = vec![
        Spec {
            id: "npm",
            title: "npm cache",
            path: home.join(".npm/_cacache"),
            detail: "Downloaded again on the next install",
            action: tool("npm", "npm cache clean --force"),
            recommended: true,
        },
        Spec {
            id: "yarn",
            title: "Yarn cache",
            path: home.join(".cache/yarn"),
            detail: "Downloaded again on the next install",
            action: tool("yarn", "yarn cache clean"),
            recommended: true,
        },
        Spec {
            id: "pip",
            title: "pip cache",
            path: home.join(".cache/pip"),
            detail: "Downloaded again on the next install",
            action: tool("pip", "pip cache purge"),
            recommended: true,
        },
        Spec {
            id: "uv",
            title: "uv cache",
            path: home.join(".cache/uv"),
            detail: "Downloaded again on the next install",
            action: tool("uv", "uv cache clean"),
            recommended: true,
        },
        Spec {
            id: "go",
            title: "Go build cache",
            path: go_cache,
            detail: "Rebuilt on the next go build",
            action: tool("go", "go clean -cache"),
            recommended: true,
        },
        Spec {
            id: "cargo-src",
            title: "Cargo unpacked sources",
            path: home.join(".cargo/registry/src"),
            detail: "Unpacked again from the local archive, no download",
            action: "Delete folder".into(),
            recommended: true,
        },
        Spec {
            id: "cargo-cache",
            title: "Cargo downloaded crates",
            path: home.join(".cargo/registry/cache"),
            detail: "Downloaded again when a project needs them",
            action: "Delete folder".into(),
            recommended: false,
        },
    ];
    let mut items: Vec<CleanItem> = specs
        .into_par_iter()
        .filter_map(|s| folder_item(s, home, true))
        .collect();

    // pnpm: only packages no project uses go, and only pnpm can tell which, so no size.
    let pnpm_store = home.join(".local/share/pnpm/store");
    if run::has_program("pnpm")
        && let Some(mut item) = folder_item(
            Spec {
                id: "pnpm",
                title: "pnpm store",
                path: pnpm_store,
                detail: "Removes only packages no project uses",
                action: "pnpm store prune".into(),
                recommended: true,
            },
            home,
            false,
        )
    {
        item.detail = "Removes only packages no project uses; pnpm reports how much".into();
        items.push(item);
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.size.unwrap_or(0)));
    items
}

/// Folders in `~/.cache` that belong to developer tools above, or that the desktop shares.
/// Only caches an installed app claims are offered at all: test browsers (Cypress, Playwright,
/// Puppeteer) live in `~/.cache` too, and removing them breaks test runs until reinstalled.
const NOT_APP_CACHES: &[&str] = &[
    "thumbnails",
    "yarn",
    "pip",
    "go-build",
    "pnpm",
    "uv",
    "fontconfig",
    "mesa_shader_cache",
    "mesa_shader_cache_db",
    "nvidia",
    "ibus",
    "tracker",
    "tracker3",
    "gstreamer-1.0",
    "appnux",
    "io.github.mindulabs.appnux",
    "dconf",
    "gvfs",
    "keyring",
    "sessions",
];

fn app_caches(home: &Path, installed: &[App]) -> Vec<CleanItem> {
    let cache = home.join(".cache");
    let Ok(entries) = fs::read_dir(&cache) else {
        return Vec::new();
    };
    let known: HashSet<String> = crate::residue::known_identifiers(installed);
    // Running programs by process name, plus every name of the apps running now (Chrome runs
    // as "chrome" but caches in "google-chrome").
    let mut running = running_names();
    let live: Vec<App> = {
        let ids: HashSet<String> = crate::procs::usage(installed)
            .into_iter()
            .map(|u| u.id)
            .collect();
        installed
            .iter()
            .filter(|a| ids.contains(&a.id))
            .cloned()
            .collect()
    };
    running.extend(crate::residue::known_identifiers(&live));
    let dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            !NOT_APP_CACHES.contains(&name.as_str())
        })
        .collect();
    let mut items: Vec<CleanItem> = dirs
        .into_par_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let size = run::disk_usage(&path);
            if size < 50 * 1024 * 1024 {
                return None;
            }
            let stem = lower
                .split(['-', '_', '.'])
                .next()
                .unwrap_or(&lower)
                .to_owned();
            // A cache in use is left alone: deleting under a running app can break it.
            if running.contains(&lower) || running.contains(&stem) {
                return None;
            }
            if !(known.contains(&lower) || known.contains(&stem)) {
                return None;
            }
            Some(CleanItem {
                id: format!("cache:{}", path.display()),
                title: name,
                path: Some(display(&path, home)),
                detail: "Rebuilt by the app".into(),
                size: Some(size),
                confidence: Confidence::High,
                needs_root: false,
                recommended: false,
                action: "Delete folder".into(),
                op: Op::Delete { path: path.clone() },
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.size.unwrap_or(0)));
    items
}

/// Lowercase names of running programs, to keep the caches they hold open.
fn running_names() -> HashSet<String> {
    let Ok(procs) = fs::read_dir("/proc") else {
        return HashSet::new();
    };
    procs
        .flatten()
        .filter_map(|p| fs::read_to_string(p.path().join("comm")).ok())
        .flat_map(|comm| {
            let name = comm.trim().to_lowercase();
            let stem = name
                .split(['-', '_', '.'])
                .next()
                .unwrap_or(&name)
                .to_owned();
            [name, stem]
        })
        .collect()
}

fn dead_shortcuts() -> Vec<CleanItem> {
    apps::dead_shortcuts()
        .into_iter()
        .map(|(path, name)| CleanItem {
            id: format!("shortcut:{}", path.display()),
            title: name,
            path: Some(display(&path, &distro::home())),
            detail: "The program it opens is gone".into(),
            size: Some(fs::metadata(&path).map(|m| m.len()).unwrap_or(0)),
            confidence: Confidence::High,
            needs_root: false,
            recommended: true,
            action: "Delete file".into(),
            op: Op::DeleteLauncher { path },
        })
        .collect()
}

fn removed_apps(home: &Path) -> Vec<CleanItem> {
    let flatpaks = apps::installed_flatpaks();
    let snaps: HashSet<String> = apps::snap_list().into_iter().map(|(n, _)| n).collect();
    let mut found = Vec::new();
    if run::has_program("flatpak")
        && let Ok(entries) = fs::read_dir(home.join(".var/app"))
    {
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !flatpaks.contains(&id) {
                found.push((e.path(), format!("Flatpak {id} is no longer installed")));
            }
        }
    }
    if run::has_program("snap")
        && let Ok(entries) = fs::read_dir(home.join("snap"))
    {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !snaps.contains(&name) {
                found.push((e.path(), format!("Snap {name} is no longer installed")));
            }
        }
    }
    found
        .into_par_iter()
        .map(|(path, detail)| CleanItem {
            id: format!("removed:{}", path.display()),
            title: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: Some(display(&path, home)),
            detail,
            size: Some(run::disk_usage(&path)),
            confidence: Confidence::High,
            needs_root: false,
            recommended: false,
            action: "Delete folder".into(),
            op: Op::Delete { path: path.clone() },
        })
        .collect()
}

fn trash(home: &Path) -> Option<CleanItem> {
    let files = home.join(".local/share/Trash/files");
    let size = run::disk_usage(&files);
    (size > 1024 * 1024).then(|| CleanItem {
        id: "trash".into(),
        title: "Trash".into(),
        path: Some("~/.local/share/Trash".into()),
        detail: "Emptying cannot be undone".into(),
        size: Some(size),
        confidence: Confidence::Medium,
        needs_root: false,
        recommended: false,
        action: "Empty Trash".into(),
        op: Op::EmptyTrash,
    })
}
