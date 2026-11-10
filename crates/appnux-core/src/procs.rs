//! Memory per app, from the cgroup scope the desktop starts each app in.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::model::{App, AppKind, ProcessUsage};

/// Proportional set size (PSS) summed over each app's processes. Only processes the user can
/// read are counted, which covers every app the user started.
pub fn usage(apps: &[App]) -> Vec<ProcessUsage> {
    let mut by_desktop: HashMap<String, &str> = HashMap::new();
    let mut by_exec: HashMap<String, &str> = HashMap::new();
    for app in apps {
        by_desktop.insert(app.desktop_id.to_lowercase(), &app.id);
        match app.kind {
            AppKind::Flatpak => {
                by_desktop.insert(format!("flatpak:{}", app.package.to_lowercase()), &app.id);
            }
            AppKind::Snap => {
                by_desktop.insert(format!("snap:{}", app.package.to_lowercase()), &app.id);
            }
            _ => {}
        }
        if let Some(exec) = &app.exec_name {
            by_exec.entry(exec.to_lowercase()).or_insert(&app.id);
        }
    }

    // A running AppImage names its own file in APPIMAGE; that tells two copies with the same
    // file name apart, which the program name cannot.
    let by_appimage: HashMap<&str, &str> = apps
        .iter()
        .filter(|a| a.kind == AppKind::AppImage)
        .filter_map(|a| Some((a.location.as_deref()?, a.id.as_str())))
        .collect();
    let mut totals: HashMap<&str, (u64, u32)> = HashMap::new();
    let Ok(procs) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for proc in procs.flatten() {
        let name = proc.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let dir = Path::new("/proc").join(pid);
        let Some(pss) = pss(&dir) else { continue };
        let scope = fs::read_to_string(dir.join("cgroup"))
            .ok()
            .and_then(|c| scope_key(&c));
        let image = appimage_of(&dir);
        let app = image
            .as_deref()
            .and_then(|p| by_appimage.get(p))
            .or_else(|| scope.as_deref().and_then(|k| by_desktop.get(k)))
            .or_else(|| {
                let exe = fs::read_link(dir.join("exe")).ok()?;
                let base = exe.file_name()?.to_string_lossy().to_lowercase();
                by_exec.get(&base)
            });
        if let Some(app) = app {
            let entry = totals.entry(app).or_default();
            entry.0 += pss;
            entry.1 += 1;
        }
    }
    totals
        .into_iter()
        .map(|(id, (memory, processes))| ProcessUsage {
            id: id.to_owned(),
            memory,
            processes,
        })
        .collect()
}

fn appimage_of(dir: &Path) -> Option<String> {
    let env = fs::read(dir.join("environ")).ok()?;
    env.split(|b| *b == 0).find_map(|var| {
        var.strip_prefix(b"APPIMAGE=")
            .map(|v| String::from_utf8_lossy(v).into_owned())
    })
}

fn pss(dir: &Path) -> Option<u64> {
    let text = fs::read_to_string(dir.join("smaps_rollup")).ok()?;
    let line = text.lines().find(|l| l.starts_with("Pss:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// The app a cgroup path names: `app-flatpak-<id>-<n>.scope`, `snap.<name>.<app>-….scope`,
/// `app-gnome-<desktop id>-<pid>.scope`, `app-<launcher>-<desktop id>@<id>.service`.
fn scope_key(cgroup: &str) -> Option<String> {
    let path = cgroup.lines().next()?.rsplit("::").next()?;
    let unit = path.rsplit('/').next()?;
    if let Some(rest) = unit.strip_prefix("app-flatpak-") {
        let id = rest.rsplit_once('-').map_or(rest, |(id, _)| id);
        return Some(format!("flatpak:{}", id.to_lowercase()));
    }
    if let Some(rest) = unit.strip_prefix("snap.") {
        return Some(format!("snap:{}", rest.split('.').next()?.to_lowercase()));
    }
    let rest = unit.strip_prefix("app-")?;
    let rest = rest.split_once('-').map(|(_, r)| r)?;
    let rest = rest
        .strip_suffix(".scope")
        .map(|r| r.rsplit_once('-').map_or(r, |(id, _)| id))
        .or_else(|| rest.split('@').next())?;
    Some(rest.replace("\\x2d", "-").to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes() {
        assert_eq!(
            scope_key("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.gimp.GIMP-1234.scope").as_deref(),
            Some("flatpak:org.gimp.gimp")
        );
        assert_eq!(
            scope_key("0::/user.slice/user@1000.service/app.slice/snap.firefox.firefox-9f2a.scope")
                .as_deref(),
            Some("snap:firefox")
        );
        assert_eq!(
            scope_key(
                "0::/user.slice/user@1000.service/app.slice/app-gnome-org.gnome.Nautilus-5521.scope"
            )
            .as_deref(),
            Some("org.gnome.nautilus")
        );
        assert_eq!(
            scope_key(
                "0::/user.slice/user@1000.service/app.slice/app-gnome-google\\x2dchrome-4242.scope"
            )
            .as_deref(),
            Some("google-chrome")
        );
    }
}
