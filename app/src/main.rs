//! Appnux: see, uninstall and clean every app on Linux. GTK4 + libadwaita front end over
//! `appnux-core`. Scans run on worker threads and report back to the main loop through `store`.

mod activity;
mod apps;
mod cleaner;
mod devshot;
mod exec;
mod overview;
mod prefs;
mod settings;
mod store;
mod ui;
mod uninstall;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use appnux_core::cleaner::ScanOptions;
use gtk::{gdk, gio, glib};

use crate::store::{Event, Shared, Store};

const APP_ID: &str = "io.github.mindulabs.Appnux";

fn main() -> glib::ExitCode {
    // `pkexec appnux --root-helper`: run the root operations read from stdin, print results.
    if std::env::args().nth(1).as_deref() == Some("--root-helper") {
        return root_helper();
    }
    let prefs = prefs::load();
    choose_renderer(prefs.hardware_acceleration);
    appnux_core::limit_threads();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| load_css());
    app.connect_activate(move |app| build(app, prefs.clone()));
    app.run()
}

fn root_helper() -> glib::ExitCode {
    use std::io::Read;
    if !appnux_core::actions::is_root() {
        eprintln!("appnux: --root-helper runs only through pkexec");
        return glib::ExitCode::FAILURE;
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return glib::ExitCode::FAILURE;
    }
    println!("{}", appnux_core::actions::helper_main(&input));
    glib::ExitCode::SUCCESS
}

/// The cairo renderer draws on the CPU and takes about half the memory of OpenGL, which this
/// app does not need: lists and a bar. GSK reads the choice from the environment, once.
fn choose_renderer(hardware_acceleration: bool) {
    if hardware_acceleration || std::env::var_os("GSK_RENDERER").is_some() {
        return;
    }
    // SAFETY: first thing in main, before GTK starts and before any other thread exists, so
    // nothing reads the environment concurrently.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("GSK_RENDERER", "cairo");
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    #[allow(deprecated)]
    provider.load_from_data(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

const PAGES: [(&str, &str, &str); 5] = [
    ("overview", "Overview", "drive-harddisk-symbolic"),
    ("apps", "Apps", "view-app-grid-symbolic"),
    ("cleaner", "Cleaner", "edit-clear-all-symbolic"),
    ("activity", "Activity", "document-open-recent-symbolic"),
    ("settings", "Settings", "emblem-system-symbolic"),
];

fn build(app: &adw::Application, prefs: prefs::Prefs) {
    settings::apply_theme(&devshot::theme_override().unwrap_or_else(|| prefs.theme.clone()));
    let store: Shared = Rc::new(Store::default());
    store.state.borrow_mut().prefs = prefs;

    let toasts = adw::ToastOverlay::new();
    let window_slot: Rc<RefCell<Option<gtk::Window>>> = Rc::default();

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hexpand(true)
        .vexpand(true)
        .build();
    let title = adw::WindowTitle::new("Overview", "");
    let (sidebar, lists) = sidebar();

    let navigate: overview::Navigate = {
        let lists = lists.clone();
        Rc::new(move |name: &str| select_page(&lists, name))
    };
    stack.add_named(
        &overview::Overview::new(&store, navigate).root,
        Some("overview"),
    );
    stack.add_named(
        &apps::Apps::new(&store, &window_slot, &toasts).root,
        Some("apps"),
    );
    stack.add_named(
        &cleaner::Cleaner::new(&store, &window_slot, &toasts).root,
        Some("cleaner"),
    );
    stack.add_named(&activity::page(&store), Some("activity"));
    stack.add_named(&settings::page(&store), Some("settings"));

    for list in &lists {
        let stack = stack.clone();
        let title = title.clone();
        let others: Vec<gtk::ListBox> = lists.iter().filter(|l| *l != list).cloned().collect();
        list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            // Pages on top, Settings at the bottom: two lists, one selection.
            for other in &others {
                other.unselect_all();
            }
            let name = row.widget_name();
            stack.set_visible_child_name(&name);
            if let Some((_, label, _)) = PAGES.iter().find(|(n, _, _)| *n == name.as_str()) {
                title.set_title(label);
            }
        });
    }
    select_page(&lists, "overview");

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    header.set_show_start_title_buttons(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&header);
    content.append(&stack);

    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    layout.append(&sidebar);
    layout.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    layout.append(&content);
    toasts.set_child(Some(&layout));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Appnux")
        .icon_name(APP_ID)
        .default_width(1240)
        .default_height(800)
        .width_request(900)
        .height_request(560)
        .content(&toasts)
        .build();
    *window_slot.borrow_mut() = Some(window.clone().upcast());
    window.present();
    {
        let lists = lists.clone();
        devshot::schedule(
            &window,
            move |page| select_page(&lists, page),
            &["overview", "apps", "cleaner", "activity", "settings"],
        );
    }

    load(&store);
    poll_usage(&store, &stack);
}

fn sidebar() -> (gtk::Box, Vec<gtk::ListBox>) {
    let nav_list = |items: &[(&str, &str, &str)]| {
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        for (name, label, icon) in items {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(name);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            line.set_margin_top(6);
            line.set_margin_bottom(6);
            line.set_margin_start(4);
            line.append(&gtk::Image::from_icon_name(icon));
            line.append(&ui::label(label, &[]));
            row.set_child(Some(&line));
            ui::pointer(&row);
            list.append(&row);
        }
        list
    };
    let pages = nav_list(&PAGES[..4]);
    let bottom = nav_list(&PAGES[4..]);
    bottom.set_margin_bottom(6);

    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    brand.set_margin_start(10);
    brand.append(&ui::logo(22));
    brand.append(&ui::label("Appnux", &["heading"]));
    // At the start, not as the title: a header bar always centres its title widget.
    let header = adw::HeaderBar::new();
    header.pack_start(&brand);
    header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_width_request(220);
    column.append(&header);
    column.append(&pages);
    column.append(&spacer);
    column.append(&bottom);
    (column, vec![pages, bottom])
}

fn select_page(lists: &[gtk::ListBox], name: &str) {
    for list in lists {
        let mut i = 0;
        while let Some(row) = list.row_at_index(i) {
            if row.widget_name() == name {
                list.select_row(Some(&row));
                return;
            }
            i += 1;
        }
    }
}

/// Startup: system facts, the disk, then apps, their footprints and the cleaner scan.
fn load(store: &Shared) {
    let s = store.clone();
    glib::spawn_future_local(async move {
        if let Ok(info) = gio::spawn_blocking(appnux_core::distro::system_info).await {
            s.update(Event::System, |st| st.system = Some(info));
        }
    });
    let s = store.clone();
    glib::spawn_future_local(async move {
        if let Ok(disk) = gio::spawn_blocking(appnux_core::disk::overview).await {
            s.update(Event::Disk, |st| st.disk = Some(disk));
        }
    });
    load_apps(store);
}

/// After something was removed: measure the disk, list apps and scan again.
pub fn reload(store: &Shared) {
    let s = store.clone();
    glib::spawn_future_local(async move {
        if let Ok(disk) = gio::spawn_blocking(appnux_core::disk::overview).await {
            s.update(Event::Disk, |st| st.disk = Some(disk));
        }
    });
    store.update(Event::ScanStarted, |s| s.groups = None);
    load_apps(store);
}

pub fn load_apps(store: &Shared) {
    if store.state.borrow().apps_loading {
        return;
    }
    store.update(Event::Apps, |s| s.apps_loading = true);
    let store = store.clone();
    glib::spawn_future_local(async move {
        let Ok(found) = gio::spawn_blocking(appnux_core::apps::discover).await else {
            store.update(Event::Apps, |s| s.apps_loading = false);
            return;
        };
        store.update(Event::Apps, |s| {
            s.apps = found.clone();
            s.apps_loading = false;
            s.footprints_loading = true;
        });
        refresh_usage(&store);
        let list = found.clone();
        if let Ok(footprints) =
            gio::spawn_blocking(move || appnux_core::residue::footprints(&list)).await
        {
            store.update(Event::Footprints, |s| {
                s.footprints = footprints.into_iter().map(|f| (f.id.clone(), f)).collect();
                s.footprints_loading = false;
            });
        }
        if store.state.borrow().groups.is_none() {
            scan(&store);
        }
    });
}

pub fn scan(store: &Shared) {
    if store.state.borrow().scanning {
        return;
    }
    store.update(Event::ScanStarted, |s| s.scanning = true);
    let (apps, keep) = {
        let s = store.state.borrow();
        (s.apps.clone(), s.prefs.journal_keep_mb)
    };
    let store = store.clone();
    glib::spawn_future_local(async move {
        let options = ScanOptions {
            journal_keep_mb: keep,
        };
        let result = gio::spawn_blocking(move || appnux_core::cleaner::scan(&apps, options)).await;
        store.update(Event::Scanned, |s| {
            s.scanning = false;
            if let Ok(groups) = result {
                s.selected = groups
                    .iter()
                    .flat_map(|g| &g.items)
                    .filter(|i| i.recommended)
                    .map(|i| i.id.clone())
                    .collect();
                s.groups = Some(groups);
            }
        });
    });
}

fn refresh_usage(store: &Shared) {
    let apps = store.state.borrow().apps.clone();
    let store = store.clone();
    glib::spawn_future_local(async move {
        if let Ok(usage) = gio::spawn_blocking(move || appnux_core::procs::usage(&apps)).await {
            store.update(Event::Usage, |s| {
                s.usage = usage.into_iter().map(|u| (u.id.clone(), u)).collect()
            });
        }
    });
}

/// Memory changes while Apps is open; nothing polls on other pages.
fn poll_usage(store: &Shared, stack: &gtk::Stack) {
    let store = store.clone();
    let stack = stack.clone();
    glib::timeout_add_seconds_local(3, move || {
        if stack.visible_child_name().as_deref() == Some("apps")
            && !store.state.borrow().apps.is_empty()
        {
            refresh_usage(&store);
        }
        glib::ControlFlow::Continue
    });
}
