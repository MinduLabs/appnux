//! Standard uninstall and Deep purge: build the plan (with the package manager's dry run),
//! show it, then run it and record every step in Activity.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use appnux_core::plan::{self, Mode, UninstallPlan};
use appnux_core::{App, Confidence};
use gtk::{gio, glib};

use appnux_core::actions::Op;

use crate::store::Shared;
use crate::ui::{self, bytes, label};

pub fn open(
    parent: &gtk::Window,
    store: &Shared,
    app: &App,
    mode: Mode,
    toasts: &adw::ToastOverlay,
) {
    let purge = mode == Mode::Purge;
    let verb = if purge { "Purge" } else { "Uninstall" };
    let title = if purge {
        format!("Deep purge {}?", app.name)
    } else {
        format!("Uninstall {}?", app.name)
    };
    let description = if purge {
        "Removes the app and deletes, for good, the files it left in your home folder. Review what will go."
    } else {
        "Removes the app. Its settings and data stay, so a reinstall picks up where you left off."
    };

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    let loading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    loading.append(&gtk::Spinner::builder().spinning(true).build());
    loading.append(&label("Checking what would be removed…", &["dim-label"]));
    body.append(&loading);

    let dialog = ui::dialog(
        parent,
        &title,
        description,
        body.upcast_ref(),
        &format!("{verb} {}", app.name),
        true,
    );
    dialog.confirm.set_sensitive(false);
    dialog.window.present();

    let (residue, running, home) = {
        let s = store.state.borrow();
        (
            s.footprints
                .get(&app.id)
                .map(|f| f.residue.clone())
                .unwrap_or_default(),
            s.usage.contains_key(&app.id),
            s.system
                .as_ref()
                .map(|i| i.home.clone())
                .unwrap_or_default(),
        )
    };
    let short = move |p: &str| {
        if !home.is_empty() && p.starts_with(&home) {
            format!("~{}", &p[home.len()..])
        } else {
            p.to_owned()
        }
    };

    let app = app.clone();
    let store = store.clone();
    let toasts = toasts.clone();
    glib::spawn_future_local(async move {
        let app_plan = app.clone();
        let Ok(plan) =
            gio::spawn_blocking(move || plan::uninstall(&app_plan, mode, residue, running)).await
        else {
            loading.set_visible(false);
            body.append(&ui::callout(
                "error",
                "Couldn't build the plan. Close this and try again.",
            ));
            return;
        };
        body.remove(&loading);
        let picked: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(
            plan.residue
                .iter()
                .filter(|r| r.confidence == Confidence::High)
                .map(|r| r.path.clone())
                .collect(),
        ));
        show_plan(
            &body,
            &plan,
            &picked,
            &short,
            &dialog.status,
            app.install_size,
        );
        dialog.confirm.set_sensitive(plan.blocked.is_none());

        let window = dialog.window.clone();
        let confirm = dialog.confirm.clone();
        let status = dialog.status.clone();
        dialog.confirm.connect_clicked(move |_| {
            let chosen = picked.borrow();
            let mut ops: Vec<(Op, u64)> = plan.ops.iter().cloned().map(|op| (op, 0)).collect();
            // The package itself frees its install size; count it on the first step.
            if let Some(first) = ops.first_mut() {
                first.1 = app.install_size;
            }
            for r in plan.residue.iter().filter(|r| chosen.contains(&r.path)) {
                ops.push((
                    Op::Delete {
                        path: r.path.clone().into(),
                    },
                    r.size,
                ));
            }
            confirm.set_sensitive(false);
            status.set_label(if plan.needs_root {
                "Waiting for your password…"
            } else {
                "Removing…"
            });
            let window = window.clone();
            crate::exec::run(
                &store,
                format!("{verb} {}", app.name),
                ops,
                &toasts,
                move |_| window.close(),
            );
        });
    });
}

fn show_plan(
    body: &gtk::Box,
    plan: &UninstallPlan,
    picked: &Rc<RefCell<HashSet<String>>>,
    short: &(impl Fn(&str) -> String + Clone + 'static),
    status: &gtk::Label,
    app_size: u64,
) {
    if let Some(why) = &plan.blocked {
        body.append(&ui::callout("error", why));
    }
    for warning in &plan.warnings {
        body.append(&ui::callout("warning", warning));
    }

    let runs = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&label("Runs", &["heading"]));
    if plan.needs_root {
        head.append(&gtk::Image::from_icon_name("dialog-password-symbolic"));
        head.append(&label("asks for your password", &["dim-label", "caption"]));
    }
    runs.append(&head);
    let commands = label(
        &plan.commands.join("\n"),
        &["monospace", "caption", "code-block"],
    );
    commands.set_selectable(true);
    commands.set_wrap(true);
    commands.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    runs.append(&commands);
    if plan.simulated && !plan.removes.is_empty() {
        let removes = label(
            &format!(
                "Dry run by the package manager removes: {}",
                plan.removes.join(", ")
            ),
            &["dim-label", "caption"],
        );
        removes.set_wrap(true);
        runs.append(&removes);
    }
    body.append(&runs);

    let update_status = {
        let plan = plan.clone();
        let picked = picked.clone();
        let status = status.clone();
        move || {
            if plan.blocked.is_some() {
                status.set_label("");
                return;
            }
            let chosen = picked.borrow();
            let freed: u64 = app_size
                + plan
                    .residue
                    .iter()
                    .filter(|r| chosen.contains(&r.path))
                    .map(|r| r.size)
                    .sum::<u64>();
            status.set_label(&format!("Frees {}", bytes(freed)));
        }
    };
    update_status();

    if plan.mode != Mode::Purge {
        return;
    }
    let files = gtk::Box::new(gtk::Orientation::Vertical, 6);
    files.append(&label("Files left behind", &["heading"]));
    if plan.residue.is_empty() {
        files.append(&label(
            "No files found in your home folder.",
            &["dim-label"],
        ));
    } else {
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        for r in &plan.residue {
            let check = gtk::CheckButton::new();
            check.set_active(picked.borrow().contains(&r.path));
            let row = adw::ActionRow::builder()
                .title(ui::escape(&short(&r.path)))
                .activatable_widget(&check)
                .build();
            row.add_css_class("monospace-title");
            row.add_prefix(&check);
            row.add_suffix(&crate::apps::confidence_badge(r.confidence));
            row.add_suffix(&ui::value(&bytes(r.size), 8));
            ui::pointer(&row);
            let path = r.path.clone();
            let picked = picked.clone();
            let update = update_status.clone();
            check.connect_toggled(move |c| {
                if c.is_active() {
                    picked.borrow_mut().insert(path.clone());
                } else {
                    picked.borrow_mut().remove(&path);
                }
                update();
            });
            list.append(&row);
        }
        files.append(&list);
        files.append(&label(
            "Ticked folders are deleted for good. Only folders marked Sure are ticked for you.",
            &["dim-label", "caption"],
        ));
    }
    body.append(&files);
}
