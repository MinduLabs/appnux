use adw::prelude::*;

use crate::prefs;
use crate::store::{Event, Shared};
use crate::ui::{self, label};

pub fn apply_theme(theme: &str) {
    adw::StyleManager::default().set_color_scheme(match theme {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });
}

fn save(store: &Shared, change: impl FnOnce(&mut prefs::Prefs)) {
    store.update(Event::Prefs, |s| {
        change(&mut s.prefs);
        prefs::save(&s.prefs);
    });
}

fn dropdown_row(
    title: &str,
    subtitle: &str,
    options: &[&str],
    selected: u32,
) -> (adw::ActionRow, gtk::DropDown) {
    let dropdown = gtk::DropDown::from_strings(options);
    dropdown.set_selected(selected);
    dropdown.set_valign(gtk::Align::Center);
    ui::pointer(&dropdown);
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    row.add_suffix(&dropdown);
    (row, dropdown)
}

fn switch_row(title: &str, subtitle: &str, active: bool) -> (adw::ActionRow, gtk::Switch) {
    let switch = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    ui::pointer(&switch);
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .activatable_widget(&switch)
        .build();
    row.add_suffix(&switch);
    ui::pointer(&row);
    (row, switch)
}

pub fn page(store: &Shared) -> gtk::Box {
    let p = store.state.borrow().prefs.clone();

    let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
    let themes = ["system", "light", "dark"];
    let current = themes.iter().position(|t| *t == p.theme).unwrap_or(0);
    let (theme_row, theme) = dropdown_row(
        "Theme",
        "System follows your desktop's light or dark setting.",
        &["System", "Light", "Dark"],
        u32::try_from(current).unwrap_or(0),
    );
    {
        let store = store.clone();
        theme.connect_selected_notify(move |d| {
            let value = themes
                .get(d.selected() as usize)
                .copied()
                .unwrap_or("system");
            apply_theme(value);
            save(&store, |p| p.theme = value.into());
        });
    }
    appearance.add(&theme_row);
    let (gpu_row, gpu) = switch_row(
        "Hardware acceleration",
        "Draws with OpenGL: smoother scrolling, about twice the memory. Applies after a restart.",
        p.hardware_acceleration,
    );
    {
        let store = store.clone();
        gpu.connect_active_notify(move |s| {
            let on = s.is_active();
            save(&store, |p| p.hardware_acceleration = on);
        });
    }
    appearance.add(&gpu_row);

    let cleaner = adw::PreferencesGroup::builder().title("Cleaner").build();
    let sizes = [50_u64, 100, 200, 500];
    let current = sizes
        .iter()
        .position(|s| *s == p.journal_keep_mb)
        .unwrap_or(1);
    let (journal_row, journal) = dropdown_row(
        "System logs to keep",
        "The systemd journal: logs Linux keeps for all services. Not Appnux's Activity.",
        &["50 MB", "100 MB", "200 MB", "500 MB"],
        u32::try_from(current).unwrap_or(1),
    );
    {
        let store = store.clone();
        journal.connect_selected_notify(move |d| {
            let mb = sizes.get(d.selected() as usize).copied().unwrap_or(100);
            save(&store, |p| p.journal_keep_mb = mb);
        });
    }
    cleaner.add(&journal_row);

    let about = adw::PreferencesGroup::builder().title("About").build();
    let values: Vec<(&str, gtk::Label)> = [
        "Version",
        "License",
        "System",
        "Package manager",
        "Desktop",
        "Kernel",
    ]
    .into_iter()
    .map(|k| {
        let v = label("—", &["dim-label"]);
        v.set_selectable(true);
        let row = adw::ActionRow::builder().title(k).build();
        row.add_suffix(&v);
        about.add(&row);
        (k, v)
    })
    .collect();
    let note = ui::callout(
        "info",
        "Uninstall, Deep purge and Clean delete for good after you confirm. Every step is listed in Activity.",
    );
    note.set_margin_top(12);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    column.append(&appearance);
    column.append(&cleaner);
    let about_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    about_box.append(&about);
    about_box.append(&note);
    column.append(&about_box);

    let store_e = store.clone();
    let fill = move || {
        let s = store_e.state.borrow();
        let Some(info) = &s.system else { return };
        for (key, label) in &values {
            label.set_label(&match *key {
                "Version" => format!("{} preview", env!("CARGO_PKG_VERSION")),
                "License" => "GPL-3.0-or-later · github.com/MinduLabs/Appnux".into(),
                "System" => info.distro.clone(),
                "Package manager" => format!(
                    "{} ({})",
                    info.backend.clone().unwrap_or_else(|| "none".into()),
                    info.family
                ),
                "Desktop" => info.desktop.clone(),
                _ => info.kernel.clone(),
            });
        }
    };
    fill();
    store.on(move |event| {
        if event == Event::System {
            fill();
        }
    });
    ui::page(None, &column, true, None)
}
