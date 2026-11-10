//! Doing what a plan says: typed operations, a guard on every path, and a root helper for the
//! few system operations. Nothing here builds a shell command.
//!
//! - User operations run as the user: a tool's own clean command, deleting or trashing a path
//!   inside the allowed folders of the home directory, emptying the Trash.
//! - Root operations go through `pkexec` to `appnux --root-helper`, which reads them as JSON,
//!   checks every value again, and runs a fixed command for each.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::distro::{self, Backend};
use crate::run;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Op {
    /// A tool's own command, as the user (`npm cache clean --force`, `flatpak uninstall`).
    Tool {
        program: String,
        args: Vec<String>,
    },
    /// Delete a file or folder for good.
    Delete {
        path: PathBuf,
    },
    EmptyTrash,
    /// Delete one `.AppImage` file in the home folder, wherever the user kept it.
    DeleteAppImage {
        path: PathBuf,
    },
    /// Delete one launcher (`.desktop`) in `~/.local/share/applications`.
    DeleteLauncher {
        path: PathBuf,
    },
    Root(RootOp),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum RootOp {
    PackageCacheClean,
    JournalVacuum {
        keep_mb: u64,
    },
    SnapRemoveRevision {
        name: String,
        revision: String,
    },
    SnapRemove {
        name: String,
        purge: bool,
    },
    /// `expect` is what the dry run showed the user; the helper refuses if it changed.
    RemovePackage {
        name: String,
        purge: bool,
        expect: Vec<String>,
    },
}

impl Op {
    /// What the step does, for previews and the Activity log.
    pub fn describe(&self) -> String {
        match self {
            Op::Tool { program, args } => format!("{program} {}", args.join(" ")),
            Op::Delete { path } => format!("Delete {}", short(path)),
            Op::EmptyTrash => "Empty Trash".into(),
            Op::DeleteAppImage { path } | Op::DeleteLauncher { path } => {
                format!("Delete {}", short(path))
            }
            Op::Root(op) => {
                let (program, args) = root_command(op, distro::backend());
                format!("pkexec {program} {}", args.join(" "))
            }
        }
    }

    pub fn needs_root(&self) -> bool {
        matches!(self, Op::Root(_))
    }
}

fn short(path: &Path) -> String {
    let home = distro::home();
    path.strip_prefix(&home).map_or_else(
        |_| path.display().to_string(),
        |p| format!("~/{}", p.display()),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub step: String,
    pub ok: bool,
    pub error: Option<String>,
    /// Bytes the step was expected to free; counted only when it succeeded.
    pub freed: u64,
}

// ---------- Path guard ----------

/// Folders inside home where Appnux may delete. Only things *inside* them, never the folder.
const ALLOWED_ROOTS: &[&str] = &[
    ".cache",
    ".config",
    ".local/share",
    ".local/state",
    ".var/app",
    "snap",
    ".npm",
    ".cargo/registry",
    ".gradle",
    "Applications",
    "AppImages",
    ".local/bin",
];

/// Never deleted, whatever matched them.
const PROTECTED: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".pki",
    ".local/share/keyrings",
    ".local/share/Trash",
    ".config/autostart",
    ".local/share/applications",
    ".local/share/icons",
    ".local/share/fonts",
    ".config/dconf",
    ".local/share/flatpak",
    ".cargo/bin",
    ".rustup",
    ".config/systemd",
    ".local/share/gnome-shell",
];

/// Why `path` may not be deleted, or `None` when it may. Pure, so it is tested with any home.
pub fn guard(path: &Path, home: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some("not an absolute path".into());
    }
    if path.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Some("the path has . or .. in it".into());
    }
    let Ok(rel) = path.strip_prefix(home) else {
        return Some("outside your home folder".into());
    };
    if rel.as_os_str().is_empty() {
        return Some("that is your home folder".into());
    }
    // A symlink anywhere above the target could point outside home.
    let mut walked = home.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        walked.push(part);
        if fs::symlink_metadata(&walked).is_ok_and(|m| m.file_type().is_symlink()) {
            return Some(format!("{} is a link", walked.display()));
        }
    }
    let rel_str = rel.to_string_lossy();
    if PROTECTED
        .iter()
        .any(|p| rel_str == *p || rel_str.starts_with(&format!("{p}/")))
    {
        return Some("a protected folder".into());
    }
    let inside_root = ALLOWED_ROOTS
        .iter()
        .any(|root| rel.starts_with(root) && rel != Path::new(root));
    // A dot folder right in home that belongs to an app (~/.mozilla), never a root itself.
    let dot_folder = parts.len() == 1
        && rel_str.starts_with('.')
        && fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
        && !ALLOWED_ROOTS
            .iter()
            .any(|root| root.split('/').next() == Some(&*rel_str))
        && ![".local", ".cache", ".config", ".var", ".cargo"].contains(&&*rel_str);
    if !inside_root && !dot_folder {
        return Some("outside the folders Appnux cleans".into());
    }
    if crate::residue::SHARED_NAMES.contains(
        &rel.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
            .as_str(),
    ) && parts.len() == 2
    {
        return Some("shared by several apps".into());
    }
    None
}

// ---------- Running ----------

/// Runs every step; a failed step is recorded and the rest go on. Root steps run together in
/// one `pkexec`, so the password is asked once.
pub fn execute(steps: Vec<(Op, u64)>) -> Vec<StepResult> {
    let home = distro::home();
    let mut results = Vec::new();
    let mut root: Vec<(RootOp, u64, String)> = Vec::new();
    for (op, size) in steps {
        let step = op.describe();
        let outcome = match op {
            Op::Root(r) => {
                root.push((r, size, step));
                continue;
            }
            Op::Tool { program, args } => run_tool(&program, &args),
            Op::Delete { path } => delete(&path, &home),
            Op::EmptyTrash => empty_trash(&home),
            Op::DeleteAppImage { path } => delete_file(&path, &home, |p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("appimage"))
            }),
            Op::DeleteLauncher { path } => delete_file(&path, &home, |p| {
                p.parent() == Some(home.join(".local/share/applications").as_path())
                    && p.extension().is_some_and(|e| e == "desktop")
            }),
        };
        results.push(result(step, outcome, size));
    }
    if !root.is_empty() {
        results.extend(run_root(root));
    }
    results
}

fn result(step: String, outcome: Result<(), String>, size: u64) -> StepResult {
    match outcome {
        Ok(()) => StepResult {
            step,
            ok: true,
            error: None,
            freed: size,
        },
        Err(e) => StepResult {
            step,
            ok: false,
            error: Some(e),
            freed: 0,
        },
    }
}

/// Programs a Tool step may run, so a crafted plan cannot run anything else.
const TOOLS: &[&str] = &["npm", "yarn", "pip", "pip3", "uv", "go", "pnpm", "flatpak"];

fn run_tool(program: &str, args: &[String]) -> Result<(), String> {
    if !TOOLS.contains(&program) {
        return Err(format!("{program} is not a tool Appnux runs"));
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run::output_checked(program, &args).map(|_| ())
}

fn delete(path: &Path, home: &Path) -> Result<(), String> {
    if let Some(why) = guard(path, home) {
        return Err(format!("Refused: {why}"));
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    // remove_dir_all never follows symlinks; a link itself is removed as a file.
    let removed = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    removed.map_err(|e| e.to_string())
}

/// One regular file in home that `fits`, with no link on the way.
fn delete_file(path: &Path, home: &Path, fits: impl Fn(&Path) -> bool) -> Result<(), String> {
    if !path.is_absolute() || !path.starts_with(home) || !fits(path) {
        return Err("Refused: not a file Appnux removes".into());
    }
    let mut walked = home.to_path_buf();
    if let Ok(rel) = path.strip_prefix(home) {
        for part in rel.parent().into_iter().flat_map(Path::components) {
            walked.push(part);
            if fs::symlink_metadata(&walked).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err("Refused: the path goes through a link".into());
            }
        }
    }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Refused: not a regular file".into());
    }
    fs::remove_file(path).map_err(|e| e.to_string())
}

/// Empties `files` and `info`, as file managers do. `expunged` is GIO's own scratch space and
/// may hold entries owned by another user, so it is left alone. One stuck item does not stop
/// the rest; the step fails only if something is left behind.
fn empty_trash(home: &Path) -> Result<(), String> {
    let trash = home.join(".local/share/Trash");
    let mut stuck = Vec::new();
    for sub in ["files", "info"] {
        let Ok(entries) = fs::read_dir(trash.join(sub)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let removed = if is_dir {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(e) = removed {
                stuck.push(format!("{}: {e}", path.display()));
            }
        }
    }
    match stuck.as_slice() {
        [] => Ok(()),
        [one] => Err(format!("1 item couldn't be removed: {one}")),
        [first, ..] => Err(format!(
            "{} items couldn't be removed, e.g. {first}",
            stuck.len()
        )),
    }
}

// ---------- Root ----------

/// Snap names, package names and revisions: nothing that a command could read as an option
/// or a path.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name.len() < 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-._:@".contains(c))
}

/// The fixed command for a root operation.
pub fn root_command(op: &RootOp, backend: Option<Backend>) -> (String, Vec<String>) {
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    match op {
        RootOp::PackageCacheClean => match backend {
            Some(Backend::Pacman) if run::has_program("paccache") => {
                ("paccache".into(), s(&["-rk1"]))
            }
            Some(Backend::Pacman) => ("pacman".into(), s(&["-Sc", "--noconfirm"])),
            Some(Backend::Dnf) => ("dnf".into(), s(&["clean", "packages"])),
            _ => ("apt-get".into(), s(&["clean"])),
        },
        RootOp::JournalVacuum { keep_mb } => (
            "journalctl".into(),
            vec![format!("--vacuum-size={keep_mb}M")],
        ),
        RootOp::SnapRemoveRevision { name, revision } => (
            "snap".into(),
            vec![
                "remove".into(),
                name.clone(),
                format!("--revision={revision}"),
            ],
        ),
        RootOp::SnapRemove { name, purge } => {
            let mut args = s(&["remove"]);
            if *purge {
                args.push("--purge".into());
            }
            args.push(name.clone());
            ("snap".into(), args)
        }
        RootOp::RemovePackage { name, purge, .. } => match backend {
            Some(Backend::Pacman) => (
                "pacman".into(),
                vec![
                    if *purge { "-Rns" } else { "-R" }.into(),
                    "--noconfirm".into(),
                    name.clone(),
                ],
            ),
            Some(Backend::Dnf) => (
                "dnf".into(),
                vec!["remove".into(), "-y".into(), name.clone()],
            ),
            _ => (
                "apt-get".into(),
                vec![
                    if *purge { "purge" } else { "remove" }.into(),
                    "-y".into(),
                    name.clone(),
                ],
            ),
        },
    }
}

fn check_root_op(op: &RootOp) -> Result<(), String> {
    match op {
        RootOp::PackageCacheClean => Ok(()),
        RootOp::JournalVacuum { keep_mb } => {
            if (10..=100_000).contains(keep_mb) {
                Ok(())
            } else {
                Err("journal size out of range".into())
            }
        }
        RootOp::SnapRemoveRevision { name, revision } => {
            if valid_name(name)
                && revision.chars().all(|c| c.is_ascii_digit())
                && !revision.is_empty()
            {
                Ok(())
            } else {
                Err("bad snap name or revision".into())
            }
        }
        RootOp::SnapRemove { name, .. } => {
            if valid_name(name) {
                Ok(())
            } else {
                Err("bad snap name".into())
            }
        }
        RootOp::RemovePackage {
            name,
            purge,
            expect,
        } => {
            if !valid_name(name) {
                return Err("bad package name".into());
            }
            if crate::apps::is_core_package(name) {
                return Err(format!("{name} is part of the core system"));
            }
            // Dry run again, as root, right before removing: refuse if it changed since the
            // user saw it, or if it now takes a core package with it.
            let mut now = crate::plan::simulate_removal(name, *purge)?;
            if let Some(core) = now.iter().find(|p| crate::apps::is_core_package(p)) {
                return Err(format!(
                    "Removing {name} would also remove {core}, which the system needs. Nothing was removed."
                ));
            }
            let mut seen = expect.clone();
            now.sort();
            seen.sort();
            if now != seen {
                return Err(format!(
                    "The package manager now wants to remove {}, not what you approved. Nothing was removed.",
                    now.join(", ")
                ));
            }
            Ok(())
        }
    }
}

pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// Inside the helper (as root): check and run each operation.
pub fn helper_main(input: &str) -> String {
    let ops: Vec<RootOp> = match serde_json::from_str(input) {
        Ok(ops) => ops,
        Err(e) => {
            return serde_json::to_string(&vec![StepResult {
                step: "read request".into(),
                ok: false,
                error: Some(e.to_string()),
                freed: 0,
            }])
            .unwrap_or_default();
        }
    };
    let backend = distro::backend();
    let results: Vec<StepResult> = ops
        .iter()
        .map(|op| {
            let (program, args) = root_command(op, backend);
            let step = format!("{program} {}", args.join(" "));
            let outcome = check_root_op(op).and_then(|()| {
                let out = Command::new(&program)
                    .args(&args)
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .env("LC_ALL", "C")
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| e.to_string())?;
                if out.status.success() {
                    Ok(())
                } else {
                    let err = String::from_utf8_lossy(&out.stderr);
                    Err(err
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("failed")
                        .trim()
                        .to_owned())
                }
            });
            result(step, outcome, 0)
        })
        .collect();
    serde_json::to_string(&results).unwrap_or_default()
}

/// From the app (as the user): one `pkexec` for all root operations when the app is installed;
/// from an AppImage, root cannot read the image's private mount, so each command gets its own
/// `pkexec` (and its own password prompt).
fn run_root(ops: Vec<(RootOp, u64, String)>) -> Vec<StepResult> {
    let cancelled = |ops: &[(RootOp, u64, String)], why: &str| {
        ops.iter()
            .map(|(_, _, step)| StepResult {
                step: step.clone(),
                ok: false,
                error: Some(why.into()),
                freed: 0,
            })
            .collect()
    };
    if !run::has_program("pkexec") {
        return cancelled(
            &ops,
            "pkexec is not installed, so Appnux can't ask for your password",
        );
    }
    for (op, _, _) in &ops {
        if let Err(e) = check_root_op_shape(op) {
            return cancelled(&ops, &e);
        }
    }
    let exe = std::env::current_exe().ok();
    let in_appimage = std::env::var_os("APPIMAGE").is_some();
    match exe {
        Some(exe) if !in_appimage => {
            let request =
                serde_json::to_string(&ops.iter().map(|(op, _, _)| op.clone()).collect::<Vec<_>>())
                    .unwrap_or_default();
            let child = Command::new("pkexec")
                .arg(&exe)
                .arg("--root-helper")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let Ok(mut child) = child else {
                return cancelled(&ops, "couldn't start pkexec");
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(request.as_bytes());
            }
            let Ok(out) = child.wait_with_output() else {
                return cancelled(&ops, "pkexec stopped");
            };
            if out.status.code() == Some(126) || out.status.code() == Some(127) {
                return cancelled(&ops, "Cancelled: the password was not given");
            }
            let parsed: Vec<StepResult> = serde_json::from_slice(&out.stdout).unwrap_or_default();
            if parsed.len() != ops.len() {
                return cancelled(&ops, String::from_utf8_lossy(&out.stderr).trim());
            }
            parsed
                .into_iter()
                .zip(&ops)
                .map(|(mut r, (_, size, step))| {
                    r.step = step.clone();
                    r.freed = if r.ok { *size } else { 0 };
                    r
                })
                .collect()
        }
        _ => ops
            .iter()
            .map(|(op, size, step)| {
                let (program, args) = root_command(op, distro::backend());
                let mut cmd = Command::new("pkexec");
                cmd.args([
                    "env",
                    "DEBIAN_FRONTEND=noninteractive",
                    "LC_ALL=C",
                    &program,
                ])
                .args(&args)
                .stdin(Stdio::null());
                let outcome = match cmd.output() {
                    Ok(out) if out.status.success() => Ok(()),
                    Ok(out) if matches!(out.status.code(), Some(126 | 127)) => {
                        Err("Cancelled: the password was not given".into())
                    }
                    Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .last()
                        .unwrap_or("failed")
                        .to_owned()),
                    Err(e) => Err(e.to_string()),
                };
                result(step.clone(), outcome, *size)
            })
            .collect(),
    }
}

/// The checks that need no root (names, ranges) run before asking for the password.
fn check_root_op_shape(op: &RootOp) -> Result<(), String> {
    match op {
        RootOp::RemovePackage { name, .. } | RootOp::SnapRemove { name, .. } => {
            if valid_name(name) {
                Ok(())
            } else {
                Err("bad name".into())
            }
        }
        other => check_root_op(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "appnux-guard-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for d in [
            ".config/vlc",
            ".cache/yarn",
            ".ssh",
            ".mozilla",
            "Documents",
            ".local/share/Trash",
            ".config/dconf",
        ] {
            let _ = fs::create_dir_all(dir.join(d));
        }
        dir
    }

    #[test]
    fn guard_allows_only_app_folders() {
        let h = home();
        assert_eq!(guard(&h.join(".config/vlc"), &h), None);
        assert_eq!(guard(&h.join(".cache/yarn"), &h), None);
        assert_eq!(guard(&h.join(".mozilla"), &h), None);
        assert!(guard(&h, &h).is_some(), "home itself");
        assert!(guard(&h.join(".config"), &h).is_some(), "a root itself");
        assert!(guard(&h.join(".cache"), &h).is_some(), "a root itself");
        assert!(guard(&h.join(".ssh"), &h).is_some(), "protected");
        assert!(guard(&h.join(".config/dconf"), &h).is_some(), "protected");
        assert!(guard(&h.join("Documents"), &h).is_some(), "user files");
        assert!(
            guard(&h.join(".local/share/Trash"), &h).is_some(),
            "the trash itself"
        );
        assert!(
            guard(Path::new("/etc/passwd"), &h).is_some(),
            "outside home"
        );
        assert!(
            guard(&h.join(".config/../Documents"), &h).is_some(),
            "dot dot"
        );
        assert!(guard(Path::new("relative/path"), &h).is_some(), "relative");
    }

    #[test]
    fn guard_refuses_paths_through_links() {
        let h = home();
        let link = h.join(".config/linked");
        let _ = fs::remove_file(&link);
        #[allow(clippy::unwrap_used)]
        std::os::unix::fs::symlink(h.join("Documents"), &link).unwrap();
        assert!(guard(&link.join("secret"), &h).is_some());
    }

    #[test]
    fn root_names_are_checked() {
        assert!(valid_name("vlc"));
        assert!(valid_name("libreoffice-calc"));
        assert!(!valid_name("--purge"));
        assert!(!valid_name("a b"));
        assert!(!valid_name("../x"));
        assert!(check_root_op(&RootOp::JournalVacuum { keep_mb: 1 }).is_err());
        assert!(
            check_root_op(&RootOp::SnapRemoveRevision {
                name: "core".into(),
                revision: "12a".into()
            })
            .is_err()
        );
    }

    #[test]
    fn delete_works_in_a_fake_home() {
        let h = home();
        let gone = h.join(".config/vlc");
        assert_eq!(delete(&gone, &h), Ok(()));
        assert!(!gone.exists());
        assert!(delete(&h.join("Documents"), &h).is_err());
        assert!(h.join("Documents").exists());
        let profile = h.join(".profile");
        #[allow(clippy::unwrap_used)]
        fs::write(&profile, b"x").unwrap();
        assert!(
            guard(&profile, &h).is_some(),
            "dot files are not app folders"
        );
    }

    #[test]
    fn empty_trash_skips_expunged() {
        let h = home();
        let trash = h.join(".local/share/Trash");
        for d in ["files/old", "info", "expunged/123"] {
            let _ = fs::create_dir_all(trash.join(d));
        }
        #[allow(clippy::unwrap_used)]
        fs::write(trash.join("info/old.trashinfo"), b"x").unwrap();
        assert_eq!(empty_trash(&h), Ok(()));
        assert!(!trash.join("files/old").exists());
        assert!(!trash.join("info/old.trashinfo").exists());
        assert!(trash.join("expunged/123").exists(), "left alone");
    }

    #[test]
    fn narrow_file_deletes() {
        let h = home();
        let image = h.join("Tool.AppImage");
        let other = h.join("notes.txt");
        let _ = fs::create_dir_all(h.join(".local/share/applications"));
        let launcher = h.join(".local/share/applications/tool.desktop");
        for f in [&image, &other, &launcher] {
            #[allow(clippy::unwrap_used)]
            fs::write(f, b"x").unwrap();
        }
        let results = execute(vec![
            (
                Op::DeleteAppImage {
                    path: image.clone(),
                },
                1,
            ),
            (
                Op::DeleteAppImage {
                    path: other.clone(),
                },
                1,
            ),
            (
                Op::DeleteLauncher {
                    path: launcher.clone(),
                },
                1,
            ),
        ]);
        // execute() uses the real home, so outside it everything is refused: nothing goes.
        assert!(results.iter().all(|r| !r.ok));
        assert!(image.exists() && other.exists() && launcher.exists());
        let fits = |p: &Path| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("appimage"))
        };
        assert!(delete_file(&other, &h, fits).is_err());
        assert_eq!(delete_file(&image, &h, fits), Ok(()));
        assert!(!image.exists());
    }
}
