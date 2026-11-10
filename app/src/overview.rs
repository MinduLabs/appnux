use adw::prelude::*;
use appnux_core::AppKind;

use crate::store::{Event, Shared};
use crate::ui::{self, Segment, SegmentedBar, bytes, label};

pub struct Overview {
    pub root: gtk::Box,
}

/// Callback to switch page by name; the window owns navigation.
pub type Navigate = std::rc::Rc<dyn Fn(&str)>;

impl Overview {
    pub fn new(store: &Shared, navigate: Navigate) -> Self {
        // Disk card
        let free = label("", &["hero", "numeric"]);
        let free_skeleton = ui::skeleton(220, 30);
        let bar = SegmentedBar::new(28);
        let bar_skeleton = ui::skeleton(-1, 28);
        bar_skeleton.set_hexpand(true);
        bar_skeleton.set_halign(gtk::Align::Fill);
        let measuring = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        measuring.append(&gtk::Spinner::builder().spinning(true).build());
        measuring.append(&label("Measuring caches…", &["dim-label", "caption"]));
        measuring.set_halign(gtk::Align::End);
        measuring.set_hexpand(true);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 4);
        heading.append(&label("Free space", &["dim-label"]));
        heading.append(&free_skeleton);
        heading.append(&free);
        top.append(&heading);
        top.append(&measuring);

        let disk_card = ui::card();
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 16);
        inner.set_margin_top(16);
        inner.set_margin_bottom(16);
        inner.set_margin_start(16);
        inner.set_margin_end(16);
        inner.append(&top);
        inner.append(&bar_skeleton);
        inner.append(&bar.bar);
        inner.append(&bar.legend);
        disk_card.append(&inner);

        // Stat tiles
        let (apps_tile, apps_value, apps_detail) = stat("view-app-grid-symbolic", "Installed apps");
        let (clean_tile, clean_value, clean_detail) =
            stat("edit-clear-all-symbolic", "Ready to clean");
        let (warn_tile, warn_value, warn_detail) =
            stat("document-open-recent-symbolic", "Not used in 3 months");
        let tiles = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        tiles.set_homogeneous(true);
        for (tile, page) in [
            (&apps_tile, "apps"),
            (&clean_tile, "cleaner"),
            (&warn_tile, "apps"),
        ] {
            tiles.append(tile);
            let go = navigate.clone();
            tile.connect_clicked(move |_| go(page));
        }

        // Lists
        let wins = adw::PreferencesGroup::builder()
            .title("Quick wins")
            .description("The largest caches that are safe to clean now.")
            .build();
        let wins_list = list();
        wins.add(&wins_list);
        let largest = adw::PreferencesGroup::builder()
            .title("Largest apps")
            .description("Package plus its config, data and cache.")
            .build();
        let largest_list = list();
        largest.add(&largest_list);
        let lists = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        lists.set_homogeneous(true);
        lists.append(&wins);
        lists.append(&largest);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.append(&disk_card);
        content.append(&tiles);
        content.append(&lists);

        let refresh = {
            let store = store.clone();
            let navigate = navigate.clone();
            move |_event: Event| {
                let s = store.state.borrow();
                // Disk
                if let Some(disk) = &s.disk {
                    free_skeleton.set_visible(false);
                    bar_skeleton.set_visible(false);
                    free.set_visible(true);
                    bar.bar.set_visible(true);
                    free.set_label(&format!("{} of {}", bytes(disk.free), bytes(disk.total)));
                    let apps: u64 = s.apps.iter().map(|a| a.install_size).sum();
                    let dev: u64 = s
                        .groups
                        .iter()
                        .flatten()
                        .filter(|g| g.id == "developer")
                        .flat_map(|g| &g.items)
                        .filter(|i| !i.path.as_deref().is_some_and(|p| p.starts_with("~/.cache")))
                        .filter_map(|i| i.size)
                        .sum();
                    let other = disk
                        .used
                        .saturating_sub(apps + disk.user_data + disk.cache + dev);
                    bar.set(vec![
                        Segment {
                            label: "Apps".into(),
                            value: apps,
                        },
                        Segment {
                            label: "App data".into(),
                            value: disk.user_data,
                        },
                        Segment {
                            label: "Caches".into(),
                            value: disk.cache,
                        },
                        Segment {
                            label: "Developer caches".into(),
                            value: dev,
                        },
                        Segment {
                            label: "Everything else".into(),
                            value: other,
                        },
                        Segment {
                            label: "Free".into(),
                            value: disk.free,
                        },
                    ]);
                } else {
                    free.set_visible(false);
                    bar.bar.set_visible(false);
                }
                measuring.set_visible(s.scanning);

                // Tiles
                apps_value.set_label(&if s.apps.is_empty() {
                    "—".into()
                } else {
                    s.apps.len().to_string()
                });
                let mut counts: Vec<(AppKind, usize)> = Vec::new();
                for kind in [
                    AppKind::System,
                    AppKind::Flatpak,
                    AppKind::Snap,
                    AppKind::AppImage,
                    AppKind::Local,
                ] {
                    let n = s.apps.iter().filter(|a| a.kind == kind).count();
                    if n > 0 {
                        counts.push((kind, n));
                    }
                }
                apps_detail.set_label(
                    &counts
                        .iter()
                        .map(|(k, n)| format!("{n} {}", crate::apps::kind_name(*k).to_lowercase()))
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
                let ready: u64 = s.picked().iter().filter_map(|i| i.size).sum();
                clean_value.set_label(&if ready > 0 {
                    bytes(ready)
                } else if s.scanning {
                    "…".into()
                } else {
                    "—".into()
                });
                clean_detail.set_label(if s.scanning {
                    "Scanning…"
                } else {
                    "Caches ticked in Cleaner"
                });
                // Unknown counts as unused: nothing says it ran.
                let cutoff = crate::store::now().saturating_sub(90 * 86_400);
                let unused: Vec<_> = s
                    .apps
                    .iter()
                    .filter(|a| s.last_used(a).is_none_or(|t| t < cutoff))
                    .collect();
                let unused_size: u64 = unused.iter().map(|a| s.total(a)).sum();
                warn_value.set_label(&if s.apps.is_empty() {
                    "—".into()
                } else {
                    unused.len().to_string()
                });
                warn_detail.set_label(&if unused.is_empty() {
                    "Every app was used lately".into()
                } else {
                    format!("{} you could free by uninstalling", bytes(unused_size))
                });

                // Quick wins
                clear(&wins_list);
                match &s.groups {
                    None => {
                        for _ in 0..5 {
                            wins_list.append(&ui::skeleton_row(false));
                        }
                    }
                    Some(groups) => {
                        let mut items: Vec<_> = groups
                            .iter()
                            .flat_map(|g| g.items.iter().map(move |i| (g.title.as_str(), i)))
                            .filter(|(_, i)| i.recommended && i.size.unwrap_or(0) > 0)
                            .collect();
                        items.sort_by_key(|(_, i)| std::cmp::Reverse(i.size));
                        if items.is_empty() {
                            wins_list.append(&empty_row("Nothing to clean right now."));
                        }
                        for (group, item) in items.into_iter().take(6) {
                            let row = nav_row(&item.title, group, item.size.unwrap_or(0), None);
                            let go = navigate.clone();
                            row.connect_activated(move |_| go("cleaner"));
                            wins_list.append(&row);
                        }
                    }
                }

                // Largest apps
                clear(&largest_list);
                if s.apps.is_empty() {
                    for _ in 0..5 {
                        largest_list.append(&ui::skeleton_row(true));
                    }
                } else {
                    let mut apps: Vec<_> = s.apps.iter().map(|a| (a, s.total(a))).collect();
                    apps.sort_by_key(|(_, t)| std::cmp::Reverse(*t));
                    for (app, total) in apps.into_iter().take(6) {
                        let icon = ui::app_icon(app.icon.as_deref(), 28);
                        let row = nav_row(
                            &app.name,
                            crate::apps::kind_name(app.kind),
                            total,
                            Some(&icon),
                        );
                        let go = navigate.clone();
                        row.connect_activated(move |_| go("apps"));
                        largest_list.append(&row);
                    }
                }
            }
        };
        refresh(Event::Apps);
        store.on(move |event| {
            if !matches!(event, Event::Usage | Event::Activity | Event::Prefs) {
                refresh(event);
            }
        });

        Overview {
            root: ui::page(None, &content, true, None),
        }
    }
}

fn stat(icon: &str, title: &str) -> (gtk::Button, gtk::Label, gtk::Label) {
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&gtk::Image::from_icon_name(icon));
    head.append(&label(title, &["dim-label"]));
    let value = label("—", &["stat-value", "numeric"]);
    let detail = label("", &["dim-label", "caption"]);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.set_margin_top(14);
    body.set_margin_bottom(14);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&head);
    body.append(&value);
    body.append(&detail);
    let tile = gtk::Button::builder().child(&body).build();
    tile.add_css_class("card");
    tile.add_css_class("stat");
    ui::pointer(&tile);
    (tile, value, detail)
}

fn list() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list
}

fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn empty_row(text: &str) -> gtk::Label {
    let l = label(text, &["dim-label"]);
    l.set_margin_top(18);
    l.set_margin_bottom(18);
    l.set_margin_start(12);
    l
}

fn nav_row(title: &str, subtitle: &str, size: u64, icon: Option<&gtk::Image>) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(ui::escape(title))
        .subtitle(ui::escape(subtitle))
        .activatable(true)
        .build();
    if let Some(icon) = icon {
        row.add_prefix(icon);
    }
    row.add_suffix(&ui::value(&bytes(size), 8));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    ui::pointer(&row);
    row
}
