//! The disk that holds the home directory, split into what takes the space.

use std::fs;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;

use crate::distro;
use crate::run;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskOverview {
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    /// `~/.config`, `~/.local/share` (without installed Flatpaks), `~/.var/app`, `~/snap`.
    pub user_data: u64,
    /// `~/.cache`.
    pub cache: u64,
    /// The volume holds snapshots (Btrfs), so deleting may not free space at once.
    pub has_snapshots: bool,
    pub filesystem: String,
}

pub fn overview() -> DiskOverview {
    let home = distro::home();
    let (total, free) = rustix::fs::statvfs(&home)
        .map(|s| (s.f_blocks * s.f_frsize, s.f_bavail * s.f_frsize))
        .unwrap_or((0, 0));
    let (mount, filesystem) = mount_of(&home);

    let share = home.join(".local/share");
    let mut data_paths: Vec<std::path::PathBuf> = vec![
        home.join(".config"),
        home.join(".var/app"),
        home.join("snap"),
    ];
    if let Ok(entries) = fs::read_dir(&share) {
        data_paths.extend(
            entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    name != "flatpak" && name != "Trash"
                })
                .map(|e| e.path()),
        );
    }
    let (user_data, cache) = rayon::join(
        || data_paths.par_iter().map(|p| run::disk_usage(p)).sum(),
        || run::disk_usage(&home.join(".cache")),
    );
    DiskOverview {
        has_snapshots: filesystem == "btrfs" && has_btrfs_snapshots(),
        mount,
        total,
        used: total.saturating_sub(free),
        free,
        user_data,
        cache,
        filesystem,
    }
}

/// The longest mount point in `/proc/self/mounts` that contains `path`.
fn mount_of(path: &Path) -> (String, String) {
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    mounts
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let _device = cols.next()?;
            let point = cols.next()?.replace("\\040", " ");
            let fs = cols.next()?.to_owned();
            path.starts_with(&point).then_some((point, fs))
        })
        .max_by_key(|(point, _)| point.len())
        .unwrap_or_else(|| ("/".into(), "unknown".into()))
}

fn has_btrfs_snapshots() -> bool {
    ["/.snapshots", "/timeshift-btrfs", "/run/timeshift"]
        .iter()
        .any(|p| fs::read_dir(p).is_ok_and(|mut d| d.next().is_some()))
}

/// Cheap check used by plans: is the home volume Btrfs with snapshots?
pub fn overview_has_snapshots() -> bool {
    let (_, filesystem) = mount_of(&distro::home());
    filesystem == "btrfs" && has_btrfs_snapshots()
}
