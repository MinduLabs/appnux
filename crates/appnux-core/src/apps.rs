//! App discovery: desktop entries mapped to the package source that owns them.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use rayon::prelude::*;

use crate::desktop::{self, DesktopEntry, IconIndex};
use crate::distro::{self, Backend};
use crate::model::{App, AppKind, Scope};
use crate::run;

/// Every app with a visible desktop entry, one row per package.
pub fn discover() -> Vec<App> {
    let entries = desktop::entries();
    let icons = IconIndex::build();
    let backend = distro::backend();

    let mut flatpaks = Vec::new();
    let mut snaps = Vec::new();
    let mut appimages = Vec::new();
    let mut system = Vec::new();
    let mut local = Vec::new();
    let user_apps = distro::home().join(".local/share/applications");
    for entry in entries {
        if entry.flatpak.is_some() {
            flatpaks.push(entry);
        } else if entry.snap.is_some() {
            snaps.push(entry);
        } else if is_appimage(&entry) {
            appimages.push(entry);
        } else if entry.path.starts_with(&user_apps) {
            local.push(entry);
        } else {
            system.push(entry);
        }
    }

    let mut apps = Vec::new();
    apps.extend(flatpak_apps(flatpaks, &icons));
    apps.extend(snap_apps(snaps, &icons));
    // A shortcut whose program is gone is not an app: the cleaner lists it (dead_shortcuts).
    apps.extend(appimages.iter().filter_map(|e| appimage_app(e, &icons)));
    add_standalone_appimages(&mut apps);
    stamp_appimage_icons(&mut apps);
    let (owned, unowned) = system_apps(system, backend, &icons);
    apps.extend(owned);
    apps.extend(
        unowned
            .iter()
            .chain(local.iter())
            .filter_map(|e| local_app(e, &icons)),
    );
    unique_ids(&mut apps);
    stamp_locations(&mut apps);
    stamp_last_used(&mut apps);
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// Ids key every lookup in the UI, so two apps must never share one (two AppImage copies with
/// the same file name in different folders did).
fn unique_ids(apps: &mut [App]) {
    let mut seen = HashSet::new();
    for app in apps.iter_mut() {
        if !seen.insert(app.id.clone()) {
            let base = app.id.clone();
            let mut n = 2;
            while !seen.insert(format!("{base}#{n}")) {
                n += 1;
            }
            app.id = format!("{base}#{n}");
        }
    }
}

/// Where each app lives on disk, so the list can say it: the Flatpak's install folder, the
/// snap's mount, the program a system package runs (links resolved, so Chrome shows
/// `/opt/google/chrome`), the AppImage file.
fn stamp_locations(apps: &mut [App]) {
    let home = distro::home();
    let path_dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let resolve = |program: &str| -> Option<String> {
        let found = if program.starts_with('/') {
            Some(std::path::PathBuf::from(program))
        } else {
            path_dirs
                .iter()
                .map(|d| d.join(program))
                .find(|p| p.is_file())
        }?;
        Some(
            fs::canonicalize(&found)
                .unwrap_or(found)
                .display()
                .to_string(),
        )
    };
    for app in apps.iter_mut() {
        match app.kind {
            AppKind::Flatpak => {
                let user = home.join(".local/share/flatpak/app").join(&app.package);
                let system = Path::new("/var/lib/flatpak/app").join(&app.package);
                app.location = Some(
                    if user.exists() { user } else { system }
                        .display()
                        .to_string(),
                );
            }
            AppKind::Snap => app.location = Some(format!("/snap/{}", app.package)),
            AppKind::System | AppKind::Local => {
                let program = app.location.clone().or_else(|| app.exec_name.clone());
                if let Some(path) = program.as_deref().and_then(resolve) {
                    app.location = Some(path);
                }
            }
            AppKind::AppImage => {}
        }
    }
}

/// When each app was last used, from two signals that need no tracking of our own:
/// - GNOME Shell's `application_state`, which records `last-seen` for apps it launched;
/// - the program file's access time. Root mounts default to `relatime`, which updates it at
///   most once a day, so it still tells "used this week" from "unused for months".
fn stamp_last_used(apps: &mut [App]) {
    let gnome = gnome_last_seen();
    let path_dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for app in apps.iter_mut() {
        let mut best: Option<(u64, &str)> = None;
        let mut offer = |time: u64, source: &'static str| {
            if best.is_none_or(|(t, _)| time > t) {
                best = Some((time, source));
            }
        };
        if let Some(seen) = gnome.get(&format!("{}.desktop", app.desktop_id)) {
            offer(*seen, "GNOME Shell");
        }
        let program = match app.kind {
            AppKind::AppImage => app.location.clone().map(std::path::PathBuf::from),
            AppKind::System | AppKind::Local => app.exec_name.as_ref().and_then(|name| {
                let direct = app
                    .location
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_absolute());
                direct.or_else(|| path_dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()))
            }),
            AppKind::Flatpak | AppKind::Snap => None,
        };
        if let Some(atime) = program
            .and_then(|p| fs::metadata(p).ok())
            .and_then(|m| m.accessed().ok())
            && let Ok(since) = atime.duration_since(std::time::UNIX_EPOCH)
        {
            offer(since.as_secs(), "program file");
        }
        if let Some((time, source)) = best {
            app.last_used = Some(time);
            app.last_used_source = Some(source.to_owned());
        }
    }
}

fn gnome_last_seen() -> HashMap<String, u64> {
    let path = distro::home().join(".local/share/gnome-shell/application_state");
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let attr = |name: &str| {
                let start = line.find(&format!("{name}=\""))? + name.len() + 2;
                let end = line[start..].find('"')? + start;
                Some(&line[start..end])
            };
            let id = attr("id")?;
            let seen: u64 = attr("last-seen")?.parse().ok()?;
            Some((id.to_owned(), seen))
        })
        .collect()
}

fn is_appimage(entry: &DesktopEntry) -> bool {
    desktop::exec_program(&entry.exec)
        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".appimage"))
        || entry.id.starts_with("appimagekit_")
}

fn base_app(entry: &DesktopEntry, kind: AppKind, package: String, icons: &IconIndex) -> App {
    let exec_name = desktop::exec_program(&entry.exec).and_then(|p| {
        Path::new(&p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });
    App {
        id: format!("{}:{package}", kind_key(kind)),
        name: entry.name.clone(),
        summary: entry.comment.clone(),
        kind,
        scope: Scope::System,
        package,
        version: String::new(),
        icon: desktop::icon_data_url(&entry.icon, icons),
        desktop_file: entry.path.display().to_string(),
        location: None,
        install_size: 0,
        protected: None,
        issue: None,
        last_used: None,
        last_used_source: None,
        identifiers: Vec::new(),
        desktop_id: entry.id.clone(),
        exec_name,
    }
}

fn kind_key(kind: AppKind) -> &'static str {
    match kind {
        AppKind::System => "system",
        AppKind::Flatpak => "flatpak",
        AppKind::Snap => "snap",
        AppKind::AppImage => "appimage",
        AppKind::Local => "local",
    }
}

// ---------- Flatpak ----------

struct FlatpakInfo {
    version: String,
    size: u64,
    user: bool,
}

fn flatpak_apps(entries: Vec<DesktopEntry>, icons: &IconIndex) -> Vec<App> {
    let info = flatpak_info();
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let id = entry.flatpak.clone()?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let mut app = base_app(&entry, AppKind::Flatpak, id.clone(), icons);
            if let Some(info) = info.get(&id) {
                app.version = info.version.clone();
                app.install_size = info.size;
                app.scope = if info.user {
                    Scope::User
                } else {
                    Scope::System
                };
            }
            app.identifiers = vec![id];
            app.exec_name = None;
            Some(app)
        })
        .collect()
}

fn flatpak_info() -> HashMap<String, FlatpakInfo> {
    let Some(out) = run::output(
        "flatpak",
        &[
            "list",
            "--app",
            "--columns=application,version,size,installation",
        ],
    ) else {
        return HashMap::new();
    };
    out.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let id = (*cols.first()?).to_owned();
            Some((
                id,
                FlatpakInfo {
                    version: cols.get(1).copied().unwrap_or("").to_owned(),
                    size: cols
                        .get(2)
                        .and_then(|s| run::parse_human_size(s))
                        .unwrap_or(0),
                    user: cols.get(3).is_some_and(|s| *s == "user"),
                },
            ))
        })
        .collect()
}

pub fn installed_flatpaks() -> HashSet<String> {
    run::output("flatpak", &["list", "--columns=application"])
        .map(|out| out.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

// ---------- Snap ----------

pub struct SnapInfo {
    pub version: String,
    pub revision: String,
    pub disabled: bool,
}

/// `snap list --all`: name → revisions.
pub fn snap_list() -> Vec<(String, SnapInfo)> {
    let Some(out) = run::output("snap", &["list", "--all"]) else {
        return Vec::new();
    };
    out.lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            Some((
                (*cols.first()?).to_owned(),
                SnapInfo {
                    version: (*cols.get(1)?).to_owned(),
                    revision: (*cols.get(2)?).to_owned(),
                    disabled: cols.last().is_some_and(|n| n.contains("disabled")),
                },
            ))
        })
        .collect()
}

pub fn snap_file(name: &str, revision: &str) -> std::path::PathBuf {
    format!("/var/lib/snapd/snaps/{name}_{revision}.snap").into()
}

fn snap_apps(entries: Vec<DesktopEntry>, icons: &IconIndex) -> Vec<App> {
    let list: HashMap<String, SnapInfo> = snap_list()
        .into_iter()
        .filter(|(_, i)| !i.disabled)
        .collect();
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.snap.clone()?;
            if !seen.insert(name.clone()) {
                return None;
            }
            let mut app = base_app(&entry, AppKind::Snap, name.clone(), icons);
            if let Some(info) = list.get(&name) {
                app.version = info.version.clone();
                app.install_size = fs::metadata(snap_file(&name, &info.revision))
                    .map(|m| m.len())
                    .unwrap_or(0);
            }
            app.identifiers = vec![name];
            Some(app)
        })
        .collect()
}

// ---------- AppImage ----------

fn appimage_app(entry: &DesktopEntry, icons: &IconIndex) -> Option<App> {
    let file = desktop::exec_program(&entry.exec).unwrap_or_default();
    let size = fs::metadata(&file).ok()?.len();
    let file_name = Path::new(&file)
        .file_name()
        .map_or_else(|| entry.id.clone(), |n| n.to_string_lossy().into_owned());
    let mut app = base_app(entry, AppKind::AppImage, file_name.clone(), icons);
    app.scope = Scope::User;
    app.install_size = size;
    app.location = Some(file);
    app.version = version_from_file_name(&file_name);
    let stem = file_name
        .split(['_', '-', '.'])
        .next()
        .unwrap_or(&file_name)
        .to_owned();
    app.identifiers = vec![stem, entry.name.replace(' ', "")];
    if !entry.wm_class.is_empty() {
        app.identifiers.push(entry.wm_class.clone());
    }
    Some(app)
}

/// An AppImage starts with an ELF header that carries `AI` and the type (1 or 2) at offset 8.
fn is_appimage_file(path: &Path) -> bool {
    use std::io::Read;
    let named = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("appimage"));
    if !named || !fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let mut head = [0u8; 11];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head[..4] == b"\x7fELF"
        && &head[8..10] == b"AI"
        && matches!(head[10], 1 | 2)
}

/// AppImages are files people download and click, kept anywhere. Searched: the home folder
/// four levels deep and `/opt` two levels, skipping hidden folders and project folders (an
/// AppImage inside a project is a build output, not an installed app), plus every AppImage
/// that is running now, wherever it lives.
fn find_appimages() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    walk_for_appimages(&distro::home(), 4, &mut found);
    walk_for_appimages(Path::new("/opt"), 2, &mut found);
    found.extend(running_appimages());
    found.sort();
    found.dedup();
    found
}

const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
];
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "snap",
    "venv",
    "site-packages",
    "proc",
];

fn walk_for_appimages(dir: &Path, depth: u32, found: &mut Vec<std::path::PathBuf>) {
    if PROJECT_MARKERS.iter().any(|m| dir.join(m).exists()) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            if is_appimage_file(&path) {
                found.push(path);
            }
        } else if kind.is_dir() && depth > 0 {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk_for_appimages(&path, depth - 1, found);
            }
        }
    }
}

/// `APPIMAGE` from the environment of running processes (only the user's own are readable).
fn running_appimages() -> Vec<std::path::PathBuf> {
    let Ok(procs) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    procs
        .flatten()
        .filter(|p| {
            p.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|p| fs::read(p.path().join("environ")).ok())
        .filter_map(|env| {
            env.split(|b| *b == 0).find_map(|var| {
                var.strip_prefix(b"APPIMAGE=")
                    .map(|v| String::from_utf8_lossy(v).into_owned())
            })
        })
        .map(std::path::PathBuf::from)
        .filter(|p| is_appimage_file(p))
        .collect()
}

/// Every AppImage carries its logo as `.DirIcon` (the AppImage spec asks for it), inside its
/// compressed image (SquashFS or DwarFS). While the app runs, the image is mounted and the
/// logo is a plain file: Appnux copies it to `~/.cache/appnux/icons` then, so the logo stays
/// after the app closes. An AppImage never started since shows the generic icon.
fn stamp_appimage_icons(apps: &mut [App]) {
    let mounts = running_appimage_mounts();
    let cache = distro::home().join(".cache/appnux/icons");
    for app in apps
        .iter_mut()
        .filter(|a| a.kind == AppKind::AppImage && a.icon.is_none())
    {
        let Some(location) = app.location.clone() else {
            continue;
        };
        let Some(key) = icon_cache_key(Path::new(&location)) else {
            continue;
        };
        let cached = cache.join(&key);
        let bytes = fs::read(&cached).ok().or_else(|| {
            let mount = mounts.get(&location)?;
            let icon = fs::read(Path::new(mount).join(".DirIcon")).ok()?;
            let _ = fs::create_dir_all(&cache).and_then(|()| fs::write(&cached, &icon));
            Some(icon)
        });
        app.icon = bytes.as_deref().and_then(icon_data_url);
    }
}

/// The file's name, size and change time: a new version of the AppImage gets its own logo.
fn icon_cache_key(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let name: String = path
        .file_name()?
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(format!("{name}-{}-{modified}", meta.len()))
}

/// AppImage file → its mount point (`APPDIR`), for every AppImage running now.
fn running_appimage_mounts() -> HashMap<String, String> {
    let Ok(procs) = fs::read_dir("/proc") else {
        return HashMap::new();
    };
    procs
        .flatten()
        .filter(|p| {
            p.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|p| fs::read(p.path().join("environ")).ok())
        .filter_map(|env| {
            let var = |name: &[u8]| {
                env.split(|b| *b == 0).find_map(|v| {
                    v.strip_prefix(name)
                        .map(|x| String::from_utf8_lossy(x).into_owned())
                })
            };
            Some((var(b"APPIMAGE=")?, var(b"APPDIR=")?))
        })
        .collect()
}

/// PNG or SVG bytes as a `data:` URL; anything else (or anything huge) is left out.
fn icon_data_url(bytes: &[u8]) -> Option<String> {
    use base64::Engine;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.windows(4).take(512).any(|w| w == b"<svg") {
        "image/svg+xml"
    } else {
        return None;
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// AppImages without a menu shortcut are apps too.
fn add_standalone_appimages(apps: &mut Vec<App>) {
    for file in find_appimages() {
        let path = file.display().to_string();
        if apps
            .iter()
            .any(|a| a.location.as_deref() == Some(path.as_str()))
        {
            continue;
        }
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let pretty = pretty_name(&name);
        apps.push(App {
            id: format!("appimage:{path}"),
            name: pretty.clone(),
            summary: String::new(),
            kind: AppKind::AppImage,
            scope: Scope::User,
            package: name.clone(),
            version: version_from_file_name(&name),
            icon: None,
            desktop_file: String::new(),
            location: Some(path),
            install_size: size,
            protected: None,
            issue: None,
            last_used: None,
            last_used_source: None,
            identifiers: vec![
                pretty.replace(' ', ""),
                pretty.to_lowercase().replace(' ', "-"),
            ],
            desktop_id: String::new(),
            exec_name: Some(name),
        });
    }
}

/// "Obsidian-1.12.7.AppImage" → "Obsidian", "Quarry_0.1.0_amd64.AppImage" → "Quarry".
fn pretty_name(file_name: &str) -> String {
    let base = &file_name[..file_name.len().saturating_sub(".AppImage".len())];
    let arch = ["x86", "x86_64", "amd64", "aarch64", "arm64", "linux", "64"];
    let words: Vec<&str> = base
        .split(['-', '_'])
        .take_while(|w| {
            !w.is_empty()
                && !w.starts_with(|c: char| c.is_ascii_digit())
                && !arch.contains(&w.to_ascii_lowercase().as_str())
        })
        .collect();
    if words.is_empty() {
        base.to_owned()
    } else {
        words.join(" ")
    }
}

fn version_from_file_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let base = lower
        .strip_suffix(".appimage")
        .map_or(name, |_| &name[..name.len() - 9]);
    base.split(['_', '-'])
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()) && part.contains('.'))
        .unwrap_or("")
        .to_owned()
}

// ---------- Local (no package) ----------

fn local_app(entry: &DesktopEntry, icons: &IconIndex) -> Option<App> {
    let program = desktop::exec_program(&entry.exec);
    if program
        .as_deref()
        .is_some_and(|p| p.starts_with('/') && !Path::new(p).exists())
    {
        return None;
    }
    let mut app = base_app(entry, AppKind::Local, entry.id.clone(), icons);
    app.scope = if entry.path.starts_with(distro::home()) {
        Scope::User
    } else {
        Scope::System
    };
    app.location = program;
    app.identifiers = common_identifiers(&app, entry);
    Some(app)
}

/// Shortcuts in your app menu (`~/.local/share/applications`) whose program was deleted: they
/// show in the menu and open nothing. The cleaner offers them.
pub fn dead_shortcuts() -> Vec<(std::path::PathBuf, String)> {
    let mine = distro::home().join(".local/share/applications");
    desktop::entries()
        .into_iter()
        .filter(|e| e.path.starts_with(&mine) && e.flatpak.is_none() && e.snap.is_none())
        .filter(|e| {
            desktop::exec_program(&e.exec)
                .is_some_and(|p| p.starts_with('/') && !Path::new(&p).exists())
        })
        .map(|e| (e.path, e.name))
        .collect()
}

// ---------- System packages (apt / pacman / dnf) ----------

struct PkgInfo {
    version: String,
    size: u64,
    essential: bool,
}

fn system_apps(
    entries: Vec<DesktopEntry>,
    backend: Option<Backend>,
    icons: &IconIndex,
) -> (Vec<App>, Vec<DesktopEntry>) {
    let Some(backend) = backend else {
        return (Vec::new(), entries);
    };
    let paths: Vec<String> = entries
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    let owners = owners(backend, &paths);
    let packages: Vec<String> = owners
        .values()
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let info = package_info(backend, &packages);
    let protected_extra = dnf_protected();

    let mut by_package: HashMap<String, App> = HashMap::new();
    let mut unowned = Vec::new();
    for entry in entries {
        let Some(package) = owners.get(&entry.path.display().to_string()).cloned() else {
            unowned.push(entry);
            continue;
        };
        // A package with several launchers is one row: the entry named like the package wins.
        let replace = by_package
            .get(&package)
            .is_none_or(|existing| entry.id == package && existing.desktop_id != package);
        if !replace {
            if let Some(existing) = by_package.get_mut(&package) {
                existing
                    .identifiers
                    .extend(common_identifiers(existing, &entry));
            }
            continue;
        }
        let mut app = base_app(&entry, AppKind::System, package.clone(), icons);
        app.location = desktop::exec_program(&entry.exec).filter(|p| p.starts_with('/'));
        if let Some(info) = info.get(&package) {
            app.version = info.version.clone();
            app.install_size = info.size;
            if info.essential {
                app.protected = Some("Marked essential by the package manager".into());
            }
        }
        if app.protected.is_none()
            && (is_core_package(&package) || protected_extra.contains(&package))
        {
            app.protected = Some("Part of the core system".into());
        }
        let mut ids = vec![package.clone()];
        ids.extend(common_identifiers(&app, &entry));
        app.identifiers = ids;
        by_package.insert(package, app);
    }
    (by_package.into_values().collect(), unowned)
}

fn common_identifiers(app: &App, entry: &DesktopEntry) -> Vec<String> {
    let mut ids = vec![entry.id.clone()];
    let parts: Vec<&str> = entry.id.split('.').collect();
    if parts.len() >= 3
        && let Some(last) = parts.last()
    {
        ids.push((*last).to_owned());
    }
    if let Some(exec) = &app.exec_name {
        ids.push(exec.clone());
    }
    if !entry.wm_class.is_empty() {
        ids.push(entry.wm_class.clone());
    }
    ids
}

/// Desktop file path → owning package.
fn owners(backend: Backend, paths: &[String]) -> HashMap<String, String> {
    if paths.is_empty() {
        return HashMap::new();
    }
    let args: Vec<&str> = paths.iter().map(String::as_str).collect();
    match backend {
        Backend::Apt => {
            let mut all = vec!["-S"];
            all.extend(&args);
            let out = run::output("dpkg-query", &all).unwrap_or_default();
            out.lines()
                .filter_map(|line| {
                    let (pkgs, path) = line.split_once(": /")?;
                    let pkg = pkgs.split(", ").next()?.split(':').next()?;
                    if pkg.starts_with("diversion by") {
                        return None;
                    }
                    Some((format!("/{path}"), pkg.to_owned()))
                })
                .collect()
        }
        Backend::Pacman => {
            let mut all = vec!["-Qo"];
            all.extend(&args);
            let out = run::output("pacman", &all).unwrap_or_default();
            out.lines()
                .filter_map(|line| {
                    let (path, rest) = line.split_once(" is owned by ")?;
                    let pkg = rest.split_whitespace().next()?;
                    Some((path.to_owned(), pkg.to_owned()))
                })
                .collect()
        }
        Backend::Dnf => {
            let mut all = vec!["-qf", "--qf", "%{NAME}\n"];
            all.extend(&args);
            let out = run::output("rpm", &all).unwrap_or_default();
            out.lines()
                .zip(paths)
                .filter(|(line, _)| !line.contains(' '))
                .map(|(line, path)| (path.clone(), line.to_owned()))
                .collect()
        }
    }
}

fn package_info(backend: Backend, packages: &[String]) -> HashMap<String, PkgInfo> {
    if packages.is_empty() {
        return HashMap::new();
    }
    let names: Vec<&str> = packages.iter().map(String::as_str).collect();
    match backend {
        Backend::Apt => {
            let mut args = vec![
                "-W",
                "-f=${Package}\t${Version}\t${Installed-Size}\t${Essential}\t${Priority}\n",
            ];
            args.extend(&names);
            let out = run::output("dpkg-query", &args).unwrap_or_default();
            out.lines()
                .filter_map(|line| {
                    let cols: Vec<&str> = line.split('\t').collect();
                    let kib: u64 = cols.get(2)?.parse().unwrap_or(0);
                    let essential = cols.get(3).is_some_and(|e| *e == "yes")
                        || cols
                            .get(4)
                            .is_some_and(|p| *p == "required" || *p == "important");
                    Some((
                        (*cols.first()?).to_owned(),
                        PkgInfo {
                            version: (*cols.get(1)?).to_owned(),
                            size: kib * 1024,
                            essential,
                        },
                    ))
                })
                .collect()
        }
        Backend::Pacman => {
            let mut args = vec!["-Qi"];
            args.extend(&names);
            let out = run::output("pacman", &args).unwrap_or_default();
            let mut map = HashMap::new();
            for block in out.split("\n\n") {
                let field = |key: &str| {
                    block.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        (k.trim() == key).then(|| v.trim().to_owned())
                    })
                };
                let Some(name) = field("Name") else { continue };
                map.insert(
                    name,
                    PkgInfo {
                        version: field("Version").unwrap_or_default(),
                        size: field("Installed Size")
                            .and_then(|s| run::parse_human_size(&s))
                            .unwrap_or(0),
                        essential: false,
                    },
                );
            }
            map
        }
        Backend::Dnf => {
            let mut args = vec!["-q", "--qf", "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SIZE}\n"];
            args.extend(&names);
            let out = run::output("rpm", &args).unwrap_or_default();
            out.lines()
                .filter_map(|line| {
                    let cols: Vec<&str> = line.split('\t').collect();
                    Some((
                        (*cols.first()?).to_owned(),
                        PkgInfo {
                            version: (*cols.get(1)?).to_owned(),
                            size: cols.get(2)?.parse().unwrap_or(0),
                            essential: false,
                        },
                    ))
                })
                .collect()
        }
    }
}

/// Packages listed in `/etc/dnf/protected.d/*.conf` (Fedora / RHEL).
fn dnf_protected() -> HashSet<String> {
    let Ok(dir) = fs::read_dir("/etc/dnf/protected.d") else {
        return HashSet::new();
    };
    dir.flatten()
        .filter_map(|f| fs::read_to_string(f.path()).ok())
        .flat_map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

const CORE_EXACT: &[&str] = &[
    "apt",
    "bash",
    "coreutils",
    "dbus",
    "dnf",
    "dpkg",
    "flatpak",
    "gdm",
    "gdm3",
    "glibc",
    "gnome-control-center",
    "gnome-session",
    "gnome-settings-daemon",
    "gnome-shell",
    "grub",
    "kwin",
    "libc6",
    "lightdm",
    "mutter",
    "nautilus",
    "dolphin",
    "thunar",
    "network-manager",
    "networkmanager",
    "pacman",
    "plasma-desktop",
    "plasma-workspace",
    "policykit-1",
    "polkit",
    "rpm",
    "sddm",
    "snapd",
    "sudo",
    "systemd",
    "ubuntu-desktop",
    "xfce4-panel",
    "xfce4-session",
    "xfdesktop4",
    "xfwm4",
    "xorg",
    "xwayland",
    "konsole",
    "gnome-terminal",
    "xfce4-terminal",
];
const CORE_PREFIX: &[&str] = &[
    "linux-image",
    "linux-headers",
    "kernel",
    "systemd-",
    "xserver-xorg",
    "libwayland",
    "gnome-shell-",
    "plasma-",
    "kde-cli-tools",
    "xfce4-settings",
];

pub fn is_core_package(name: &str) -> bool {
    CORE_EXACT.contains(&name) || CORE_PREFIX.iter().any(|p| name.starts_with(p))
}

/// Sizes of packages by name, for the dashboard's "Apps" share when only names are known.
pub fn total_install_size(apps: &[App]) -> u64 {
    apps.par_iter().map(|a| a.install_size).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_in_appimage_name() {
        assert_eq!(
            version_from_file_name("Quarry_0.1.0_amd64.AppImage"),
            "0.1.0"
        );
        assert_eq!(version_from_file_name("Obsidian-1.6.7.AppImage"), "1.6.7");
        assert_eq!(version_from_file_name("tool.AppImage"), "");
    }

    #[test]
    fn appimage_names() {
        assert_eq!(pretty_name("Obsidian-1.12.7.AppImage"), "Obsidian");
        assert_eq!(pretty_name("Quarry_0.1.0_amd64.AppImage"), "Quarry");
        assert_eq!(pretty_name("Zalo.AppImage"), "Zalo");
        assert_eq!(pretty_name("Cursor-2.0.63-x86_64.AppImage"), "Cursor");
    }

    #[test]
    fn core_packages() {
        assert!(is_core_package("systemd"));
        assert!(is_core_package("linux-image-6.8.0-138-generic"));
        assert!(!is_core_package("vlc"));
    }
}
