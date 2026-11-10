//! Development aid: with `APPNUX_SNAPSHOT_DIR=<dir>`, the app renders every page in light and
//! dark to PNG files once the startup scans are done, then quits. Wayland lets no outside tool
//! capture a window, so the app draws itself.

use std::path::PathBuf;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

/// The theme a snapshot run uses, applied before the window shows.
pub fn theme_override() -> Option<String> {
    std::env::var("APPNUX_SNAPSHOT_THEME").ok()
}

pub fn schedule(
    window: &adw::ApplicationWindow,
    select: impl Fn(&str) + 'static,
    pages: &'static [&'static str],
) {
    let Some(dir) = std::env::var_os("APPNUX_SNAPSHOT_DIR").map(PathBuf::from) else {
        return;
    };
    let window = window.clone();
    let delay = std::env::var("APPNUX_SNAPSHOT_DELAY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    glib::timeout_add_seconds_local_once(delay, move || {
        // One theme per run (APPNUX_SNAPSHOT_THEME): restyling a live window mid-run leaves
        // the widget paintable empty.
        let dark = adw::StyleManager::default().is_dark();
        let shots: Vec<(String, bool)> = pages.iter().map(|p| ((*p).to_owned(), dark)).collect();
        step(window, select, dir, shots, 0);
    });
}

fn step(
    window: adw::ApplicationWindow,
    select: impl Fn(&str) + 'static,
    dir: PathBuf,
    shots: Vec<(String, bool)>,
    i: usize,
) {
    let Some((page, dark)) = shots.get(i).cloned() else {
        window.close();
        return;
    };
    select(&page);
    glib::timeout_add_local_once(Duration::from_millis(700), move || {
        let path = dir.join(format!(
            "{page}-{}.png",
            if dark { "dark" } else { "light" }
        ));
        if let Err(err) = capture(&window, &path) {
            eprintln!("appnux: snapshot {}: {err}", path.display());
        }
        step(window, select, dir, shots, i + 1);
    });
}

fn capture(window: &adw::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    // A window does not paint through WidgetPaintable; its content does.
    let content = window.content().ok_or("no content")?;
    let (w, h) = (content.width(), content.height());
    let paintable = gtk::WidgetPaintable::new(Some(&content));
    let snapshot = gtk::Snapshot::new();
    // The window paints its own background, which the content does not include.
    let bg: (u8, u8, u8) = if adw::StyleManager::default().is_dark() {
        (0x24, 0x24, 0x24)
    } else {
        (0xfa, 0xfa, 0xfa)
    };
    let color = gtk::gdk::RGBA::new(
        f32::from(bg.0) / 255.0,
        f32::from(bg.1) / 255.0,
        f32::from(bg.2) / 255.0,
        1.0,
    );
    #[allow(clippy::cast_precision_loss)]
    snapshot.append_color(
        &color,
        &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
    );
    paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
    let node = snapshot
        .to_node()
        .ok_or_else(|| format!("nothing drawn ({w}x{h}, mapped={})", content.is_mapped()))?;
    let renderer = window.renderer().ok_or("no renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}
