//! The cleaner page: caches only (docs/CLEANER.md). Sections collapse; ticks pick what goes.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use appnux_core::cleaner::CleanItem;

use appnux_core::actions::Op;

use crate::store::{Event, Shared};
use crate::ui::{self, bytes, label};

pub struct Cleaner {
    pub root: gtk::Box,
}

impl Cleaner {
    pub fn new(
        store: &Shared,
        window: &Rc<RefCell<Option<gtk::Window>>>,
        toasts: &adw::ToastOverlay,
    ) -> Self {
        let found = label("Not scanned yet", &["dim-label"]);
        found.set_hexpand(true);
        let scan = ui::tool_button("view-refresh-symbolic", "Scan");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.append(&found);
        toolbar.append(&scan);

        let sections = gtk::ListBox::new();
        sections.add_css_class("boxed-list");
        sections.set_selection_mode(gtk::SelectionMode::None);
        sections.set_valign(gtk::Align::Start);

        let summary = label("Tick what you want to clean.", &["dim-label"]);
        summary.set_hexpand(true);
        let clean = gtk::Button::with_label("Review and clean…");
        clean.add_css_class("suggested-action");
        clean.set_sensitive(false);
        ui::pointer(&clean);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.append(&summary);
        footer.append(&clean);

        {
            let store = store.clone();
            scan.connect_clicked(move |_| crate::scan(&store));
        }
        {
            let store = store.clone();
            let window = window.clone();
            let toasts = toasts.clone();
            clean.connect_clicked(move |_| {
                if let Some(parent) = window.borrow().as_ref() {
                    preview(parent, &store, &toasts);
                }
            });
        }

        let headers: Rc<RefCell<Vec<Header>>> = Rc::default();
        // Set while code changes check boxes, so their handlers do not write back.
        let syncing: Rc<std::cell::Cell<bool>> = Rc::default();
        let store_e = store.clone();
        let page_sections = sections.clone();
        let render = move |event: Event| {
            let s = store_e.state.borrow();
            scan.set_sensitive(!s.scanning);
            if let Some(content) = scan.child().and_downcast::<adw::ButtonContent>() {
                content.set_label(if s.scanning { "Scanning…" } else { "Scan" });
            }
            let picked = s.picked();
            let picked_size: u64 = picked.iter().filter_map(|i| i.size).sum();
            clean.set_sensitive(!picked.is_empty());
            summary.set_label(&if picked.is_empty() {
                "Tick what you want to clean.".into()
            } else {
                format!(
                    "{} · {} selected",
                    bytes(picked_size),
                    ui::plural(picked.len(), "item", "items")
                )
            });
            if event == Event::Selection {
                syncing.set(true);
                for header in headers.borrow().iter() {
                    header.sync(&s.selected);
                }
                syncing.set(false);
                return;
            }
            while let Some(child) = sections.first_child() {
                sections.remove(&child);
            }
            headers.borrow_mut().clear();
            let Some(groups) = &s.groups else {
                found.set_label(if s.scanning {
                    "Scanning…"
                } else {
                    "Not scanned yet"
                });
                for _ in 0..5 {
                    sections.append(&ui::skeleton_row(false));
                }
                return;
            };
            let total: u64 = groups
                .iter()
                .flat_map(|g| &g.items)
                .filter_map(|i| i.size)
                .sum();
            found.set_label(&format!(
                "Found {} in {}",
                bytes(total),
                ui::plural(groups.len(), "section", "sections")
            ));
            if groups.is_empty() {
                sections.append(&label(
                    "All clean. Nothing to remove right now.",
                    &["dim-label"],
                ));
            }
            syncing.set(true);
            for group in groups {
                let (row, header) = section(&store_e, group, &syncing);
                header.sync(&s.selected);
                sections.append(&row);
                headers.borrow_mut().push(header);
            }
            syncing.set(false);
        };
        render(Event::Scanned);
        store.on(move |event| {
            if matches!(
                event,
                Event::ScanStarted | Event::Scanned | Event::Selection
            ) {
                render(event);
            }
        });

        Cleaner {
            root: ui::page(
                Some(toolbar.upcast_ref()),
                &page_sections,
                true,
                Some(footer.upcast_ref()),
            ),
        }
    }
}

/// The widgets of one section that follow the selection.
struct Header {
    ids: Vec<String>,
    all: gtk::CheckButton,
    count: gtk::Label,
    checks: Vec<(String, gtk::CheckButton)>,
}

impl Header {
    fn sync(&self, selected: &std::collections::HashSet<String>) {
        let ticked = self.ids.iter().filter(|id| selected.contains(*id)).count();
        self.all.set_active(ticked == self.ids.len() && ticked > 0);
        self.all
            .set_inconsistent(ticked > 0 && ticked < self.ids.len());
        self.count.set_label(&if ticked > 0 {
            format!("{ticked} of {}", self.ids.len())
        } else {
            ui::plural(self.ids.len(), "item", "items")
        });
        for (id, check) in &self.checks {
            check.set_active(selected.contains(id));
        }
    }
}

fn section(
    store: &Shared,
    group: &appnux_core::cleaner::CleanGroup,
    syncing: &Rc<std::cell::Cell<bool>>,
) -> (adw::ExpanderRow, Header) {
    let size: u64 = group.items.iter().filter_map(|i| i.size).sum();
    let row = adw::ExpanderRow::builder()
        .title(ui::escape(&group.title))
        .subtitle(ui::escape(&group.description))
        .build();
    ui::pointer(&row);

    let ids: Vec<String> = group.items.iter().map(|i| i.id.clone()).collect();
    let all = gtk::CheckButton::new();
    all.set_valign(gtk::Align::Center);
    all.set_tooltip_text(Some("Select all in this section"));
    ui::pointer(&all);
    {
        let store = store.clone();
        let ids = ids.clone();
        let syncing = syncing.clone();
        all.connect_toggled(move |c| {
            if syncing.get() {
                return;
            }
            let on = c.is_active();
            store.update(Event::Selection, |s| {
                for id in &ids {
                    if on {
                        s.selected.insert(id.clone());
                    } else {
                        s.selected.remove(id);
                    }
                }
            });
        });
    }
    row.add_prefix(&all);
    let count = label("", &["dim-label", "caption"]);
    count.set_valign(gtk::Align::Center);
    row.add_action(&count);
    let total = ui::value(&if size > 0 { bytes(size) } else { "—".into() }, 8);
    total.add_css_class("heading");
    total.set_valign(gtk::Align::Center);
    row.add_action(&total);

    let mut checks = Vec::new();
    for item in &group.items {
        let (item_row, check) = item_row(store, item, syncing);
        row.add_row(&item_row);
        checks.push((item.id.clone(), check));
    }
    (
        row,
        Header {
            ids,
            all,
            count,
            checks,
        },
    )
}

fn item_row(
    store: &Shared,
    item: &CleanItem,
    syncing: &Rc<std::cell::Cell<bool>>,
) -> (adw::ActionRow, gtk::CheckButton) {
    let check = gtk::CheckButton::new();
    let subtitle = match &item.path {
        Some(p) => format!("{} · {p}", item.detail),
        None => item.detail.clone(),
    };
    let row = adw::ActionRow::builder()
        .title(ui::escape(&item.title))
        .subtitle(ui::escape(&subtitle))
        .activatable_widget(&check)
        .build();
    row.add_prefix(&check);
    ui::pointer(&row);
    if item.needs_root {
        let key = gtk::Image::from_icon_name("dialog-password-symbolic");
        key.set_tooltip_text(Some("Asks for your password"));
        row.add_suffix(&key);
    }
    row.add_suffix(&ui::value(&item.size.map_or("—".into(), bytes), 8));
    let store = store.clone();
    let id = item.id.clone();
    let syncing = syncing.clone();
    check.connect_toggled(move |c| {
        if syncing.get() {
            return;
        }
        let on = c.is_active();
        store.update(Event::Selection, |s| {
            if on {
                s.selected.insert(id.clone());
            } else {
                s.selected.remove(&id);
            }
        });
    });
    (row, check)
}

fn preview(parent: &gtk::Window, store: &Shared, toasts: &adw::ToastOverlay) {
    let (items, has_snapshots) = {
        let s = store.state.borrow();
        (s.picked(), s.disk.as_ref().is_some_and(|d| d.has_snapshots))
    };
    // What the preview prints is what runs: the operation's own description.
    let step = |i: &CleanItem| i.op.describe();
    let is_delete = |i: &CleanItem| matches!(i.op, Op::Delete { .. } | Op::EmptyTrash);
    let total: u64 = items.iter().filter_map(|i| i.size).sum();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    if has_snapshots {
        body.append(&ui::callout(
            "warning",
            "This disk keeps snapshots, so the space may not come back at once.",
        ));
    }
    let groups: [(&str, &str, Vec<&CleanItem>); 3] = [
        (
            "dialog-password-symbolic",
            "Runs with your password",
            items.iter().filter(|i| i.needs_root).collect(),
        ),
        (
            "utilities-terminal-symbolic",
            "Cleaned by the tool itself",
            items
                .iter()
                .filter(|i| matches!(i.op, Op::Tool { .. }))
                .collect(),
        ),
        (
            "edit-delete-symbolic",
            "Deleted for good",
            items.iter().filter(|i| is_delete(i)).collect(),
        ),
    ];
    for (icon, title, list) in groups {
        if list.is_empty() {
            continue;
        }
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.append(&gtk::Image::from_icon_name(icon));
        let t = label(title, &["heading"]);
        t.set_hexpand(true);
        head.append(&t);
        head.append(&label(
            &bytes(list.iter().filter_map(|i| i.size).sum()),
            &["dim-label", "numeric"],
        ));
        let rows = gtk::ListBox::new();
        rows.add_css_class("boxed-list");
        rows.set_selection_mode(gtk::SelectionMode::None);
        for item in list {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.set_margin_top(8);
            line.set_margin_bottom(8);
            line.set_margin_start(12);
            line.set_margin_end(12);
            let text = label(&step(item), &["monospace", "caption"]);
            text.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            text.set_hexpand(true);
            text.set_selectable(true);
            line.append(&text);
            line.append(&ui::value(&item.size.map_or("—".into(), bytes), 8));
            rows.append(&line);
        }
        body.append(&head);
        body.append(&rows);
    }

    let dialog = ui::dialog(
        parent,
        &format!("Clean {}?", bytes(total)),
        "Everything below is deleted for good: caches, temporary files and the leftovers you ticked. Check the list before you go on.",
        body.upcast_ref(),
        &format!("Clean {}", bytes(total)),
        true,
    );
    let window = dialog.window.clone();
    let confirm = dialog.confirm.clone();
    let status = dialog.status.clone();
    let store = store.clone();
    let toasts = toasts.clone();
    let needs_root = items.iter().any(|i| i.needs_root);
    let ops: Vec<(Op, u64)> = items
        .iter()
        .map(|i| (i.op.clone(), i.size.unwrap_or(0)))
        .collect();
    let count = items.len();
    dialog.confirm.connect_clicked(move |_| {
        confirm.set_sensitive(false);
        status.set_label(if needs_root {
            "Waiting for your password…"
        } else {
            "Cleaning…"
        });
        let window = window.clone();
        crate::exec::run(
            &store,
            format!("Clean {}", ui::plural(count, "item", "items")),
            ops.clone(),
            &toasts,
            move |_| window.close(),
        );
    });
    dialog.window.present();
}
