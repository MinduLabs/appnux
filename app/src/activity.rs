use adw::prelude::*;

use crate::store::{Event, Shared};
use crate::ui::{self, bytes, label};

pub fn page(store: &Shared) -> gtk::Box {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_valign(gtk::Align::Start);
    let empty = adw::StatusPage::builder()
        .icon_name("document-open-recent-symbolic")
        .title("Nothing yet")
        .description("Uninstalls and cleanups show up here with every step they ran.")
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&empty);
    content.append(&list);

    let store_e = store.clone();
    let render = move || {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let s = store_e.state.borrow();
        empty.set_visible(s.activity.is_empty());
        list.set_visible(!s.activity.is_empty());
        for entry in &s.activity {
            let row = adw::ExpanderRow::builder()
                .title(ui::escape(&entry.title))
                .subtitle(ui::escape(&format!(
                    "{} · {}",
                    when(entry.time),
                    ui::plural(entry.steps.len(), "step", "steps")
                )))
                .build();
            ui::pointer(&row);
            let failed = entry.failed();
            let badge = if failed == 0 {
                label("Done", &["badge", "badge-sure"])
            } else {
                label(&format!("{failed} failed"), &["badge", "badge-likely"])
            };
            badge.set_valign(gtk::Align::Center);
            row.add_action(&badge);
            let freed = ui::value(&bytes(entry.freed), 8);
            freed.add_css_class("heading");
            freed.set_valign(gtk::Align::Center);
            row.add_action(&freed);
            let text: Vec<String> = entry
                .steps
                .iter()
                .map(|s| match &s.error {
                    None => format!("✓ {}", s.step),
                    Some(e) => format!("✗ {} — {e}", s.step),
                })
                .collect();
            let steps = label(
                &text.join(
                    "
",
                ),
                &["monospace", "caption"],
            );
            steps.set_selectable(true);
            steps.set_wrap(true);
            steps.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            steps.set_margin_top(10);
            steps.set_margin_bottom(10);
            steps.set_margin_start(14);
            steps.set_margin_end(14);
            row.add_row(&steps);
            list.append(&row);
        }
    };
    render();
    store.on(move |event| {
        if event == Event::Activity {
            render();
        }
    });
    ui::page(None, &content, true, None)
}

fn when(unix: u64) -> String {
    gtk::glib::DateTime::from_unix_local(i64::try_from(unix).unwrap_or(0))
        .and_then(|d| d.format("%d %b %Y, %H:%M"))
        .map_or_else(|_| String::new(), |s| s.to_string())
}
