//! Running system tools and measuring directories.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use rayon::prelude::*;

/// Variables an AppImage's start-up hooks set for its bundled GTK. Tools we run belong to the
/// system, so they get the session's own environment back.
const APPIMAGE_VARS: &[&str] = &[
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
    "GTK_DATA_PREFIX",
    "GTK_THEME",
    "GTK_EXE_PREFIX",
    "GTK_PATH",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
    "GIO_MODULE_DIR",
    "GSETTINGS_SCHEMA_DIR",
    "GI_TYPELIB_PATH",
    "GSK_RENDERER",
];

/// `XDG_DATA_DIRS` as the desktop session set it. Inside an AppImage the GTK hook prepends
/// `$APPDIR/usr/share:/usr/share`, which hides some launchers behind others of the same name
/// (a Flatpak shadowed by a system package) and lists the AppImage's own entry.
pub fn session_data_dirs() -> String {
    let raw = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty());
    let raw = raw.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let appdir = std::env::var_os("APPDIR").map(|a| a.to_string_lossy().into_owned());
    strip_appimage_dirs(&raw, appdir.as_deref())
}

fn strip_appimage_dirs(raw: &str, appdir: Option<&str>) -> String {
    let Some(appdir) = appdir else {
        return raw.to_owned();
    };
    let mut parts: Vec<&str> = raw.split(':').collect();
    if parts.first().is_some_and(|p| p.starts_with(appdir)) {
        parts.remove(0);
        if parts
            .first()
            .is_some_and(|p| p.trim_end_matches('/') == "/usr/share")
        {
            parts.remove(0);
        }
    }
    parts.retain(|p| !p.starts_with(appdir));
    parts.join(":")
}

fn command(program: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args).env("LC_ALL", "C").stdin(Stdio::null());
    if std::env::var_os("APPDIR").is_some() {
        for var in APPIMAGE_VARS {
            cmd.env_remove(var);
        }
        cmd.env("XDG_DATA_DIRS", session_data_dirs());
    }
    cmd
}

/// Stdout of a tool, or `None` when it is missing or fails to start. A non-zero exit still
/// returns stdout: `dpkg -S` exits 1 when one of many paths is unowned.
pub fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = command(program, args).stderr(Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Stdout on success, stderr (first line) on failure.
pub fn output_checked(program: &str, args: &[&str]) -> Result<String, String> {
    let out = command(program, args)
        .output()
        .map_err(|err| format!("{program} is not available: {err}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().next().unwrap_or("failed").trim().to_owned())
}

pub fn has_program(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Bytes a tree really takes on disk: allocated blocks, hard links counted once, symlinks not
/// followed, other filesystems not entered.
pub fn disk_usage(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    let seen = Mutex::new(HashSet::new());
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    own + walk(path, meta.dev(), &seen)
}

fn walk(dir: &Path, dev: u64, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| {
            // DirEntry::metadata does not follow symlinks.
            let Ok(meta) = entry.metadata() else {
                return 0;
            };
            if meta.dev() != dev {
                return 0;
            }
            let mut size = meta.blocks() * 512;
            if meta.nlink() > 1 && !meta.is_dir() {
                let fresh = seen
                    .lock()
                    .map(|mut set| set.insert((meta.dev(), meta.ino())))
                    .unwrap_or(true);
                if !fresh {
                    size = 0;
                }
            }
            if meta.is_dir() {
                size += walk(&entry.path(), dev, seen);
            }
            size
        })
        .sum()
}

/// `1.2 GB` (Flatpak, decimal), `3.4GB` (Docker, decimal), `1.2G` (journalctl, binary),
/// `2 MiB` (pacman, binary) → bytes.
pub fn parse_human_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(text.len());
    let number: f64 = text[..split].replace(',', ".").parse().ok()?;
    let unit: String = text[split..]
        .chars()
        .filter(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_lowercase();
    let exp = match unit.chars().next() {
        None | Some('b') => 0,
        Some('k') => 1,
        Some('m') => 2,
        Some('g') => 3,
        Some('t') => 4,
        Some(_) => return None,
    };
    let binary = unit.contains('i') || unit.len() == 1 && exp > 0;
    let base: f64 = if binary { 1024.0 } else { 1000.0 };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((number * base.powi(exp)).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undoes_the_appimage_data_dirs() {
        let hooked = "/tmp/.mount_x/usr/share:/usr/share:/home/u/.local/share/flatpak/exports/share:/usr/share";
        assert_eq!(
            strip_appimage_dirs(hooked, Some("/tmp/.mount_x")),
            "/home/u/.local/share/flatpak/exports/share:/usr/share"
        );
        assert_eq!(strip_appimage_dirs("/a:/b", None), "/a:/b");
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_human_size("850 MB"), Some(850_000_000));
        assert_eq!(parse_human_size("12 kB"), Some(12_000));
        assert_eq!(parse_human_size("3.4GB"), Some(3_400_000_000));
        assert_eq!(parse_human_size("1.0G"), Some(1_073_741_824));
        assert_eq!(parse_human_size("2 MiB"), Some(2_097_152));
        assert_eq!(parse_human_size("0B"), Some(0));
        assert_eq!(parse_human_size("3.8\u{a0}MB"), Some(3_800_000));
        assert_eq!(parse_human_size("3.8?MB"), Some(3_800_000));
        assert_eq!(parse_human_size("354,3 MB"), Some(354_300_000));
        assert_eq!(parse_human_size("abc"), None);
    }
}
