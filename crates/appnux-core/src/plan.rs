//! Uninstall plans: what would run and what would be removed, before anything happens.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::actions::{Op, RootOp};

use crate::apps;
use crate::distro::{self, Backend};
use crate::model::{App, AppKind, Residue, Scope};
use crate::run;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Standard,
    Purge,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPlan {
    /// The removal itself; residue is added by the caller from what the user ticks.
    #[serde(skip)]
    pub ops: Vec<crate::actions::Op>,
    pub app_id: String,
    pub mode: Mode,
    pub commands: Vec<String>,
    pub needs_root: bool,
    /// Packages the package manager would remove, from a dry run. Empty when not simulated.
    pub removes: Vec<String>,
    pub simulated: bool,
    pub blocked: Option<String>,
    pub residue: Vec<Residue>,
    pub warnings: Vec<String>,
}

pub fn uninstall(app: &App, mode: Mode, residue: Vec<Residue>, running: bool) -> UninstallPlan {
    let purge = mode == Mode::Purge;
    let mut plan = UninstallPlan {
        ops: Vec::new(),
        app_id: app.id.clone(),
        mode,
        commands: Vec::new(),
        needs_root: false,
        removes: Vec::new(),
        simulated: false,
        blocked: app
            .protected
            .clone()
            .map(|why| format!("{} can't be removed: {why}.", app.name)),
        residue: if purge { residue } else { Vec::new() },
        warnings: Vec::new(),
    };
    if running {
        plan.warnings.push(format!(
            "{} is running. Close it before you uninstall.",
            app.name
        ));
    }
    match app.kind {
        AppKind::System => system(app, purge, &mut plan),
        AppKind::Flatpak => {
            // flatpak asks for the password itself (polkit) for system-wide apps.
            let mut args = vec!["uninstall".to_owned(), "-y".into()];
            args.push(
                if app.scope == Scope::User {
                    "--user"
                } else {
                    "--system"
                }
                .into(),
            );
            if purge {
                args.push("--delete-data".into());
            }
            args.push(app.package.clone());
            plan.ops.push(Op::Tool {
                program: "flatpak".into(),
                args,
            });
            plan.needs_root = app.scope == Scope::System;
        }
        AppKind::Snap => {
            plan.ops.push(Op::Root(RootOp::SnapRemove {
                name: app.package.clone(),
                purge,
            }));
            plan.needs_root = true;
        }
        AppKind::AppImage => {
            if let Some(file) = app.location.as_deref().filter(|f| Path::new(f).exists()) {
                plan.ops.push(Op::DeleteAppImage { path: file.into() });
            }
            if !app.desktop_file.is_empty() {
                plan.ops.push(Op::DeleteLauncher {
                    path: app.desktop_file.clone().into(),
                });
            }
        }
        AppKind::Local => {
            plan.ops.push(Op::DeleteLauncher {
                path: app.desktop_file.clone().into(),
            });
            plan.warnings.push("No package manager owns this app. Only its launcher is removed; its files stay where they are.".into());
        }
    }
    plan.commands = plan.ops.iter().map(Op::describe).collect();
    if crate::disk::overview_has_snapshots() {
        plan.warnings.push(
            "This disk keeps snapshots, so the space may not come back until they expire.".into(),
        );
    }
    plan
}

/// The packages the package manager would remove with `pkg`, from its own dry run.
pub fn simulate_removal(pkg: &str, purge: bool) -> Result<Vec<String>, String> {
    let (program, args): (&str, Vec<&str>) = match distro::backend() {
        Some(Backend::Apt) => (
            "apt-get",
            vec!["-s", if purge { "purge" } else { "remove" }, pkg],
        ),
        Some(Backend::Pacman) => (
            "pacman",
            vec![
                if purge { "-Rnsp" } else { "-Rp" },
                "--print-format",
                "%n",
                pkg,
            ],
        ),
        Some(Backend::Dnf) => ("dnf", vec!["remove", "--assumeno", pkg]),
        None => return Err("No supported package manager found.".into()),
    };
    match run::output_checked(program, &args) {
        Ok(out) => Ok(parse_simulation(program, &out)),
        // dnf exits 1 after --assumeno even when the plan printed; read stdout anyway.
        Err(_) if program == "dnf" => {
            let names = parse_simulation(program, &run::output(program, &args).unwrap_or_default());
            if names.is_empty() {
                Err("dnf printed no plan".into())
            } else {
                Ok(names)
            }
        }
        Err(err) => Err(format!("Couldn't simulate the removal: {err}")),
    }
}

fn system(app: &App, purge: bool, plan: &mut UninstallPlan) {
    plan.needs_root = true;
    let pkg = app.package.as_str();
    match simulate_removal(pkg, purge) {
        Ok(removes) => {
            plan.simulated = true;
            plan.removes = removes;
        }
        Err(err) => {
            // Without a dry run there is nothing to hold the helper to: refuse.
            plan.blocked.get_or_insert(err);
        }
    }
    plan.ops.push(Op::Root(RootOp::RemovePackage {
        name: pkg.into(),
        purge,
        expect: plan.removes.clone(),
    }));
    let core: Vec<&str> = plan
        .removes
        .iter()
        .filter(|p| apps::is_core_package(p))
        .map(String::as_str)
        .collect();
    if plan.blocked.is_none() && !core.is_empty() {
        plan.blocked = Some(format!(
            "Removing {} would also remove {}, which the system needs.",
            app.name,
            core.join(", ")
        ));
    }
    let extra = plan.removes.iter().filter(|p| *p != pkg).count();
    if extra > 0 {
        plan.warnings.push(format!(
            "{extra} other package(s) would be removed with it. Check the list."
        ));
    }
}

fn parse_simulation(program: &str, out: &str) -> Vec<String> {
    match program {
        "apt-get" => out
            .lines()
            .filter_map(|l| l.strip_prefix("Remv ").or_else(|| l.strip_prefix("Purg ")))
            .filter_map(|l| l.split_whitespace().next())
            .map(|n| n.split(':').next().unwrap_or(n).to_owned())
            .collect(),
        "pacman" => out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => {
            let mut removing = false;
            let mut names = Vec::new();
            for line in out.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Removing") {
                    removing = true;
                    continue;
                }
                if removing {
                    if trimmed.is_empty() || trimmed.ends_with(':') && !trimmed.contains(' ') {
                        if trimmed.is_empty() {
                            break;
                        }
                        continue;
                    }
                    if let Some(name) = trimmed.split_whitespace().next() {
                        names.push(name.to_owned());
                    }
                }
            }
            names
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_simulation() {
        let out = "NOTE: This is only a simulation!\nRemv vlc [3.0.16-1build7]\nPurg vlc-bin [3.0.16-1build7]\n";
        assert_eq!(parse_simulation("apt-get", out), vec!["vlc", "vlc-bin"]);
    }
}
