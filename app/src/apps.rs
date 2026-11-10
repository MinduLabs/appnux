use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use appnux_core::{App, AppKind, Confidence};

use crate::store::{Event, Shared, now};
use crate::ui::{self, Segment, SegmentedBar, ago, bytes, label};

pub fn kind_name(kind: AppKind) -> &'static str {
    match kind {
        AppKind::System => "System",
        AppKind::Flatpak => "Flatpak",
        AppKind::Snap => "Snap",
        AppKind::AppImage => "AppImage",
        AppKind::Local => "Local",
    }
}

const SOURCES: [&str; 6] = [
    "All sources",
    "System",
    "Flatpak",
    "Snap",
    "AppImage",
    "Local",
];
const UNUSED: [(&str, u64); 4] = [
    ("Used any time", 0),
    ("Not used in 30 days", 30),
    ("Not used in 3 months", 90),
    ("Not used in a year", 365),
];
const SORTS: [&str; 4] = [
    "Sort by name",
    "Sort by size",
    "Sort by last used",
    "Sort by memory",
];

struct RowParts {
    row: adw::ActionRow,
    used: gtk::Label,
    size: gtk::Label,
    memory: gtk::Label,
}

pub struct Apps {
    pub root: gtk::Box,
}

impl Apps {
    pub fn new(
        store: &Shared,
        window: &Rc<RefCell<Option<gtk::Window>>>,
        toasts: &adw::ToastOverlay,
    ) -> Self {
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search apps")
            .width_request(220)
            .build();
        let source = gtk::DropDown::from_strings(&SOURCES);
        let unused = gtk::DropDown::from_strings(&UNUSED.map(|(l, _)| l));
        let sort = gtk::DropDown::from_strings(&SORTS);
        for d in [&source, &unused, &sort] {
            d.set_valign(gtk::Align::Center);
            ui::pointer(d);
        }
        let count = label("", &["dim-label", "numeric"]);
        count.set_hexpand(true);
        count.set_xalign(1.0);
        let refresh = ui::tool_button("view-refresh-symbolic", "Refresh");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for w in [
            search.upcast_ref::<gtk::Widget>(),
            source.upcast_ref(),
            unused.upcast_ref(),
            sort.upcast_ref(),
            count.upcast_ref(),
            refresh.upcast_ref(),
        ] {
            toolbar.append(w);
        }

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_valign(gtk::Align::Start);
        // Shown when no row is visible: skeletons while apps load, then "No apps match".
        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let skeletons = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for _ in 0..10 {
            skeletons.append(&ui::skeleton_row(true));
        }
        let no_match = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("No apps match")
            .description("Try another name, source or time.")
            .build();
        no_match.set_visible(false);
        placeholder.append(&skeletons);
        placeholder.append(&no_match);
        list.set_placeholder(Some(&placeholder));
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .child(&list)
            .build();

        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.set_width_request(340);
        detail.add_css_class("card");
        // Hidden, not collapsed: a hidden card takes no space, so the list fills the column.
        detail.set_visible(false);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        body.append(&scroller);
        body.append(&detail);

        let rows: Rc<RefCell<HashMap<String, RowParts>>> = Rc::default();
        let selected: Rc<RefCell<Option<String>>> = Rc::default();

        // Filter and sort read the store, so a new scan or a new choice only needs invalidate.
        {
            let store = store.clone();
            let (search, source, unused) = (search.clone(), source.clone(), unused.clone());
            list.set_filter_func(move |row| {
                let s = store.state.borrow();
                let Some(app) = s.app(row.widget_name().as_str()) else {
                    return true;
                };
                let needle = search.text().to_lowercase();
                if !needle.is_empty()
                    && !app.name.to_lowercase().contains(&needle)
                    && !app.package.to_lowercase().contains(&needle)
                {
                    return false;
                }
                let picked = source.selected() as usize;
                if picked > 0 && SOURCES.get(picked) != Some(&kind_name(app.kind)) {
                    return false;
                }
                let days = UNUSED
                    .get(unused.selected() as usize)
                    .map_or(0, |(_, d)| *d);
                if days > 0 {
                    let cutoff = now().saturating_sub(days * 86_400);
                    // Unknown counts as unused: nothing says it ran.
                    if s.last_used(app).is_some_and(|t| t >= cutoff) {
                        return false;
                    }
                }
                true
            });
        }
        {
            let store = store.clone();
            let sort = sort.clone();
            list.set_sort_func(move |a, b| {
                let s = store.state.borrow();
                let (Some(x), Some(y)) = (
                    s.app(a.widget_name().as_str()),
                    s.app(b.widget_name().as_str()),
                ) else {
                    return gtk::Ordering::Equal;
                };
                let order = match sort.selected() {
                    1 => s.total(y).cmp(&s.total(x)),
                    2 => s.last_used(y).cmp(&s.last_used(x)),
                    3 => {
                        let m = |a: &App| s.usage.get(&a.id).map_or(0, |u| u.memory);
                        m(y).cmp(&m(x))
                    }
                    _ => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
                };
                order.into()
            });
        }
        {
            let list_f = list.clone();
            search.connect_search_changed(move |_| list_f.invalidate_filter());
            for d in [&source, &unused] {
                let list_f = list.clone();
                d.connect_selected_notify(move |_| list_f.invalidate_filter());
            }
            let list_s = list.clone();
            sort.connect_selected_notify(move |_| list_s.invalidate_sort());
        }

        // Selecting a row opens the detail card.
        {
            let store = store.clone();
            let detail = detail.clone();
            let revealer = detail.clone();
            let selected = selected.clone();
            let window = window.clone();
            let toasts = toasts.clone();
            list.connect_row_selected(move |_, row| {
                let id = row.map(|r| r.widget_name().to_string());
                *selected.borrow_mut() = id.clone();
                match id {
                    Some(id) => {
                        fill_detail(&detail, &store, &id, &window, &toasts, &revealer);
                        revealer.set_visible(true);
                    }
                    None => revealer.set_visible(false),
                }
            });
        }

        {
            let store_r = store.clone();
            refresh.connect_clicked(move |_| crate::load_apps(&store_r));
        }

        // Keep rows in step with the store.
        {
            let store_e = store.clone();
            let list = list.clone();
            let rows = rows.clone();
            let detail = detail.clone();
            let revealer = detail.clone();
            let selected = selected.clone();
            let window = window.clone();
            let toasts = toasts.clone();
            store.on(move |event| match event {
                Event::Apps => {
                    let built = build_rows(&store_e, &window, &toasts);
                    let mut map = rows.borrow_mut();
                    // Every row goes, not only the ones in the map: a row the map lost would
                    // stay in the list forever and show up twice after each refresh.
                    while let Some(row) = list.row_at_index(0) {
                        list.remove(&row);
                    }
                    map.clear();
                    let fresh: Vec<adw::ActionRow> = built
                        .into_iter()
                        .map(|(id, parts)| {
                            let row = parts.row.clone();
                            map.insert(id, parts);
                            row
                        })
                        .collect();
                    drop(map);
                    for row in &fresh {
                        list.append(row);
                    }
                    let s = store_e.state.borrow();
                    count.set_label(&format!("{} installed", s.apps.len()));
                    let loading = s.apps_loading && s.apps.is_empty();
                    skeletons.set_visible(loading);
                    no_match.set_visible(!loading);
                }
                Event::Footprints | Event::Usage => {
                    let s = store_e.state.borrow();
                    for (id, parts) in rows.borrow().iter() {
                        let Some(app) = s.app(id) else { continue };
                        update_values(parts, app, &s);
                    }
                    drop(s);
                    list.invalidate_sort();
                    list.invalidate_filter();
                    if event == Event::Footprints
                        && let Some(id) = selected.borrow().clone()
                    {
                        fill_detail(&detail, &store_e, &id, &window, &toasts, &revealer);
                    }
                }
                _ => {}
            });
        }

        Apps {
            root: ui::page(Some(toolbar.upcast_ref()), &body, false, None),
        }
    }
}

fn build_rows(
    store: &Shared,
    window: &Rc<RefCell<Option<gtk::Window>>>,
    toasts: &adw::ToastOverlay,
) -> Vec<(String, RowParts)> {
    let s = store.state.borrow();
    let home = s
        .system
        .as_ref()
        .map(|i| i.home.clone())
        .unwrap_or_default();
    s.apps
        .iter()
        .map(|app| {
            // Where it lives is the first thing to know before removing it.
            let subtitle = match &app.location {
                Some(path) => format!("{} · {}", kind_name(app.kind), short_home(path, &home)),
                None => kind_name(app.kind).to_owned(),
            };
            let row = adw::ActionRow::builder()
                .title(ui::escape(&app.name))
                .subtitle(ui::escape(&subtitle))
                .subtitle_lines(1)
                .activatable(true)
                .build();
            row.set_widget_name(&app.id);
            row.add_prefix(&ui::app_icon(app.icon.as_deref(), 32));
            ui::pointer(&row);
            if let Some(issue) = &app.issue {
                let warn = gtk::Image::from_icon_name("dialog-warning-symbolic");
                warn.add_css_class("warning");
                warn.set_tooltip_text(Some(issue));
                row.add_suffix(&warn);
            }
            let used = label("", &["dim-label"]);
            used.set_width_chars(14);
            let size = ui::value("", 9);
            let memory = ui::value("", 9);
            row.add_suffix(&used);
            row.add_suffix(&size);
            row.add_suffix(&memory);
            let remove = gtk::Button::from_icon_name("user-trash-symbolic");
            remove.add_css_class("flat");
            remove.set_valign(gtk::Align::Center);
            remove.set_tooltip_text(Some(if app.protected.is_some() {
                "Protected system component"
            } else {
                "Uninstall…"
            }));
            remove.set_sensitive(app.protected.is_none());
            ui::pointer(&remove);
            {
                let store = store.clone();
                let window = window.clone();
                let toasts = toasts.clone();
                let app = app.clone();
                remove.connect_clicked(move |_| {
                    if let Some(parent) = window.borrow().as_ref() {
                        crate::uninstall::open(
                            parent,
                            &store,
                            &app,
                            appnux_core::plan::Mode::Standard,
                            &toasts,
                        );
                    }
                });
            }
            row.add_suffix(&remove);
            let parts = RowParts {
                row,
                used,
                size,
                memory,
            };
            update_values(&parts, app, &s);
            (app.id.clone(), parts)
        })
        .collect()
}

fn update_values(parts: &RowParts, app: &App, s: &crate::store::State) {
    let running = s.usage.get(&app.id);
    parts.used.set_label(&if running.is_some() {
        "Running".into()
    } else {
        ago(s.last_used(app))
    });
    if running.is_some() {
        parts.used.remove_css_class("dim-label");
        parts.used.add_css_class("success");
    } else {
        parts.used.remove_css_class("success");
        parts.used.add_css_class("dim-label");
    }
    let total = s.total(app);
    let known = s.footprints.contains_key(&app.id) || !s.footprints_loading;
    parts.size.set_label(&if !known {
        "…".into()
    } else if total > 0 {
        bytes(total)
    } else {
        "—".into()
    });
    parts
        .memory
        .set_label(&running.map_or("—".into(), |u| bytes(u.memory)));
}

fn fill_detail(
    detail: &gtk::Box,
    store: &Shared,
    id: &str,
    window: &Rc<RefCell<Option<gtk::Window>>>,
    toasts: &adw::ToastOverlay,
    revealer: &gtk::Box,
) {
    while let Some(child) = detail.first_child() {
        detail.remove(&child);
    }
    let s = store.state.borrow();
    let Some(app) = s.app(id).cloned() else {
        return;
    };
    let footprint = s.footprints.get(id).cloned();
    let usage = s.usage.get(id).cloned();
    let home = s
        .system
        .as_ref()
        .map(|i| i.home.clone())
        .unwrap_or_default();
    let backend = s.system.as_ref().and_then(|i| i.backend.clone());
    let total = s.total(&app);
    let used = s.last_used(&app);
    drop(s);
    let short = |p: &str| {
        if !home.is_empty() && p.starts_with(&home) {
            format!("~{}", &p[home.len()..])
        } else {
            p.to_owned()
        }
    };

    // Header
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.append(&ui::app_icon(app.icon.as_deref(), 48));
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    let name = label(&app.name, &["title-3"]);
    name.set_wrap(true);
    titles.append(&name);
    if !app.summary.is_empty() {
        let summary = label(&app.summary, &["dim-label", "caption"]);
        summary.set_wrap(true);
        summary.set_lines(2);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        titles.append(&summary);
    }
    let source = match (app.kind, &backend) {
        (AppKind::System, Some(b)) => format!("System · {b}"),
        (kind, _) => kind_name(kind).to_owned(),
    };
    titles.append(&label(&source, &["dim-label", "caption"]));
    head.append(&titles);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("flat");
    close.set_valign(gtk::Align::Start);
    ui::pointer(&close);
    let r = revealer.clone();
    close.connect_clicked(move |_| r.set_visible(false));
    head.append(&close);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&head);
    if let Some(issue) = &app.issue {
        body.append(&ui::callout("warning", issue));
    }
    if let Some(why) = &app.protected {
        body.append(&ui::callout(
            "safe",
            &format!("Protected: {why}. Appnux won't remove it."),
        ));
    }

    // On disk
    let disk = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let t = label("On disk", &["heading"]);
    t.set_hexpand(true);
    line.append(&t);
    line.append(&label(
        &if total > 0 {
            bytes(total)
        } else {
            "—".into()
        },
        &["heading", "numeric"],
    ));
    disk.append(&line);
    let bar = SegmentedBar::new(8);
    let f = footprint.clone();
    bar.set(vec![
        Segment {
            label: "App".into(),
            value: app.install_size,
        },
        Segment {
            label: "Config".into(),
            value: f.as_ref().map_or(0, |f| f.config),
        },
        Segment {
            label: "Data".into(),
            value: f.as_ref().map_or(0, |f| f.data),
        },
        Segment {
            label: "Cache".into(),
            value: f.as_ref().map_or(0, |f| f.cache),
        },
    ]);
    bar.legend.set_max_children_per_line(2);
    disk.append(&bar.bar);
    disk.append(&bar.legend);
    body.append(&disk);

    // Last used and memory
    let facts = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    facts.set_homogeneous(true);
    let used_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    used_box.append(&label("Last used", &["heading"]));
    let used_label = label(
        &if usage.is_some() {
            "Running now".into()
        } else {
            ago(used)
        },
        &[if usage.is_some() {
            "success"
        } else {
            "dim-label"
        }],
    );
    used_label.set_tooltip_text(Some("Estimated from GNOME's app history, the program file's access time and the app's own folders."));
    used_box.append(&used_label);
    let mem_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    mem_box.append(&label("Memory", &["heading"]));
    mem_box.append(&label(
        &usage.as_ref().map_or("Not running".into(), |u| {
            format!(
                "{} · {}",
                bytes(u.memory),
                ui::plural(u.processes as usize, "process", "processes")
            )
        }),
        &["dim-label"],
    ));
    facts.append(&used_box);
    facts.append(&mem_box);
    body.append(&facts);

    // Details
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(12)
        .build();
    let mut rows = vec![
        ("Package", app.package.clone()),
        (
            "Version",
            if app.version.is_empty() {
                "—".into()
            } else {
                app.version.clone()
            },
        ),
    ];
    if let Some(location) = &app.location {
        rows.push(("Installed at", short(location)));
    }
    for (i, (k, v)) in rows.iter().enumerate() {
        let key = label(k, &["dim-label"]);
        let val = label(v, &["monospace", "caption"]);
        val.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        val.set_hexpand(true);
        val.set_tooltip_text(Some(v));
        val.set_selectable(true);
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let row = i as i32;
        grid.attach(&key, 0, row, 1, 1);
        grid.attach(&val, 1, row, 1, 1);
    }
    let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
    details.append(&label("Details", &["heading"]));
    details.append(&grid);
    if let Some(location) = app.location.clone() {
        let open = ui::tool_button("folder-open-symbolic", "Open folder");
        open.set_halign(gtk::Align::Start);
        open.connect_clicked(move |_| open_folder(&location));
        details.append(&open);
    }
    body.append(&details);

    // Files in the home folder
    let files = gtk::Box::new(gtk::Orientation::Vertical, 6);
    files.append(&label("Files in your home folder", &["heading"]));
    match &footprint {
        Some(f) if !f.residue.is_empty() => {
            for r in &f.residue {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let path = label(&short(&r.path), &["monospace", "caption"]);
                path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                path.set_hexpand(true);
                path.set_tooltip_text(Some(&r.path));
                line.append(&path);
                line.append(&confidence_badge(r.confidence));
                line.append(&ui::value(&bytes(r.size), 8));
                files.append(&line);
            }
        }
        _ => files.append(&label("Nothing found.", &["dim-label"])),
    }
    body.append(&files);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&body)
        .build();
    detail.append(&scroller);

    // Actions
    let uninstall = gtk::Button::with_label("Uninstall…");
    let purge = gtk::Button::with_label("Deep purge…");
    purge.add_css_class("destructive-action");
    for b in [&uninstall, &purge] {
        b.set_hexpand(true);
        b.set_sensitive(app.protected.is_none());
        ui::pointer(b);
    }
    if app.protected.is_some() {
        uninstall.set_tooltip_text(Some("Protected system component"));
        purge.set_tooltip_text(Some("Protected system component"));
    }
    for (button, mode) in [
        (&uninstall, appnux_core::plan::Mode::Standard),
        (&purge, appnux_core::plan::Mode::Purge),
    ] {
        let store = store.clone();
        let window = window.clone();
        let toasts = toasts.clone();
        let app = app.clone();
        button.connect_clicked(move |_| {
            if let Some(parent) = window.borrow().as_ref() {
                crate::uninstall::open(parent, &store, &app, mode, &toasts);
            }
        });
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_top(12);
    actions.set_margin_bottom(12);
    actions.set_margin_start(16);
    actions.set_margin_end(16);
    actions.append(&uninstall);
    actions.append(&purge);
    detail.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    detail.append(&actions);
}

pub fn confidence_badge(confidence: Confidence) -> gtk::Label {
    let (text, class, help) = match confidence {
        Confidence::High => (
            "Sure",
            "badge-sure",
            "Named exactly after this app or confirmed by its package manager.",
        ),
        Confidence::Medium => (
            "Likely",
            "badge-likely",
            "Named like this app. Check it before removing.",
        ),
        Confidence::Low => (
            "Unsure",
            "badge-unsure",
            "Shared with other apps or loosely matched. Not ticked by default.",
        ),
    };
    let badge = label(text, &["badge", class]);
    badge.set_tooltip_text(Some(help));
    badge.set_valign(gtk::Align::Center);
    badge
}

fn short_home(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

/// Opens the folder an app lives in: the folder itself for a Flatpak or a snap, the folder
/// that holds the file for a program or an AppImage.
fn open_folder(location: &str) {
    let path = std::path::Path::new(location);
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let uri = gtk::gio::File::for_path(folder).uri();
    if let Err(err) =
        gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
    {
        eprintln!("appnux: couldn't open {}: {err}", folder.display());
    }
}
