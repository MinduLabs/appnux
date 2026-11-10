//! Building blocks shared by every page: layout, labels, skeletons, dialogs, the disk bar.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use base64::Engine;
use gtk::{gdk, glib};

/// One column width for every page: toolbar, content and footer line up.
pub const PAGE_MAX: i32 = 1200;
const GUTTER: i32 = 24;

fn column(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    let clamp = adw::Clamp::builder()
        .maximum_size(PAGE_MAX)
        .tightening_threshold(PAGE_MAX)
        .child(child)
        .build();
    clamp.set_margin_start(GUTTER);
    clamp.set_margin_end(GUTTER);
    clamp
}

/// A page: an optional toolbar, the content, an optional footer, each in the page column.
/// `scroll` wraps the content in one scroll area; a page with its own scrolling list passes false.
pub fn page(
    toolbar: Option<&gtk::Widget>,
    content: &impl IsA<gtk::Widget>,
    scroll: bool,
    footer: Option<&gtk::Widget>,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(toolbar) = toolbar {
        let bar = column(toolbar);
        bar.set_margin_top(8);
        bar.set_margin_bottom(8);
        root.append(&bar);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }
    let body = column(content);
    body.set_margin_top(20);
    body.set_margin_bottom(20);
    body.set_vexpand(true);
    if scroll {
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        root.append(&scrolled);
    } else {
        root.append(&body);
    }
    if let Some(footer) = footer {
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let bar = column(footer);
        bar.set_margin_top(10);
        bar.set_margin_bottom(10);
        root.append(&bar);
        bar.add_css_class("footer");
    }
    root
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    for c in classes {
        label.add_css_class(c);
    }
    label
}

/// A right-aligned number column.
pub fn value(text: &str, width: i32) -> gtk::Label {
    let label = label(text, &["numeric"]);
    label.set_xalign(1.0);
    label.set_width_chars(width);
    label
}

/// Everything clickable shows a pointer.
pub fn pointer(widget: &impl IsA<gtk::Widget>) {
    widget.set_cursor_from_name(Some("pointer"));
}

/// A small button with an icon and a label, for toolbars.
pub fn tool_button(icon: &str, text: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name(icon)
                .label(text)
                .build(),
        )
        .valign(gtk::Align::Center)
        .build();
    pointer(&button);
    button
}

pub fn card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    card
}

/// A placeholder block while data loads.
pub fn skeleton(width: i32, height: i32) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    block.add_css_class("skeleton");
    block.set_size_request(width, height);
    block.set_halign(gtk::Align::Start);
    block.set_valign(gtk::Align::Center);
    block
}

/// A list row placeholder: icon, two lines, a value on the right.
pub fn skeleton_row(icon: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(12);
    row.set_margin_bottom(12);
    row.set_margin_start(12);
    row.set_margin_end(12);
    if icon {
        row.append(&skeleton(32, 32));
    }
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 6);
    lines.set_hexpand(true);
    lines.append(&skeleton(180, 12));
    lines.append(&skeleton(110, 10));
    row.append(&lines);
    row.append(&skeleton(56, 12));
    row
}

pub fn bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss)]
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < units.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{n} B"),
        _ if size < 10.0 => format!("{size:.1} {}", units[unit]),
        _ => format!("{size:.0} {}", units[unit]),
    }
}

/// "Today", "3 days ago": coarse on purpose, the source is an estimate.
pub fn ago(unix: Option<u64>) -> String {
    let Some(t) = unix else {
        return "Unknown".into();
    };
    match crate::store::now().saturating_sub(t) / 86_400 {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        d if d < 30 => format!("{d} days ago"),
        d if d < 60 => "1 month ago".into(),
        d if d < 365 => format!("{} months ago", d / 30),
        d if d < 730 => "1 year ago".into(),
        d => format!("{} years ago", d / 365),
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Row titles and subtitles are Pango markup: "Tom & Jerry" must not break a row.
pub fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

/// The core sends icons as `data:` URLs; decode them into a texture.
pub fn app_icon(data_url: Option<&str>, size: i32) -> gtk::Image {
    let texture = data_url
        .and_then(|url| url.split_once(";base64,"))
        .and_then(|(_, data)| base64::engine::general_purpose::STANDARD.decode(data).ok())
        .and_then(|raw| gdk::Texture::from_bytes(&glib::Bytes::from_owned(raw)).ok());
    let image = match texture {
        Some(t) => gtk::Image::from_paintable(Some(&t)),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    image.set_pixel_size(size);
    image
}

pub fn logo(size: i32) -> gtk::Image {
    let texture = gdk::Texture::from_bytes(&glib::Bytes::from_static(include_bytes!(
        "../../packaging/linux/icons/128x128.png"
    )))
    .ok();
    let image = gtk::Image::from_paintable(texture.as_ref());
    image.set_pixel_size(size);
    image
}

/// A modal dialog: title, description, body, then Cancel and the confirming button.
/// Cancel has the focus, so Enter never confirms by accident.
pub struct Dialog {
    pub window: gtk::Window,
    pub confirm: gtk::Button,
    pub status: gtk::Label,
}

pub fn dialog(
    parent: &gtk::Window,
    title: &str,
    description: &str,
    body: &gtk::Widget,
    confirm: &str,
    destructive: bool,
) -> Dialog {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .default_width(600)
        .resizable(false)
        .title(title)
        .build();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(true);
    header.set_title_widget(Some(&gtk::Label::new(None)));
    header.add_css_class("flat");

    let heading = label(title, &["title-2"]);
    heading.set_wrap(true);
    let text = label(description, &["dim-label"]);
    text.set_wrap(true);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(body)
        .build();

    let status = label("", &["dim-label", "numeric"]);
    status.set_hexpand(true);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label(confirm);
    ok.add_css_class(if destructive {
        "destructive-action"
    } else {
        "suggested-action"
    });
    pointer(&cancel);
    pointer(&ok);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&status);
    buttons.append(&cancel);
    buttons.append(&ok);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_bottom(20);
    content.append(&heading);
    content.append(&text);
    content.append(&scroller);
    content.append(&buttons);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&content);
    window.set_child(Some(&root));

    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    window.set_default_widget(Some(&cancel));
    cancel.grab_focus();
    Dialog {
        window,
        confirm: ok,
        status,
    }
}

/// A callout: an icon and wrapped text on a tinted background.
pub fn callout(kind: &str, text: &str) -> gtk::Box {
    let icon = match kind {
        "warning" => "dialog-warning-symbolic",
        "error" => "dialog-error-symbolic",
        "safe" => "security-high-symbolic",
        _ => "dialog-information-symbolic",
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("callout");
    row.add_css_class(kind);
    let image = gtk::Image::from_icon_name(icon);
    image.set_valign(gtk::Align::Start);
    let text = label(text, &[]);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.set_hexpand(true);
    text.set_selectable(true);
    row.append(&image);
    row.append(&text);
    row
}

// ---------- Disk bar ----------

/// The validated palette from the web UI, light and dark steps.
fn series(i: usize, dark: bool) -> (f64, f64, f64, f64) {
    let light = [
        (0x2a, 0x78, 0xd6),
        (0xeb, 0x68, 0x34),
        (0x1b, 0xaf, 0x7a),
        (0xed, 0xa1, 0x00),
        (0xb4, 0xb4, 0xbc),
    ];
    let darks = [
        (0x39, 0x87, 0xe5),
        (0xd9, 0x59, 0x26),
        (0x19, 0x9e, 0x70),
        (0xc9, 0x85, 0x00),
        (0x55, 0x55, 0x5c),
    ];
    match (light.get(i), darks.get(i)) {
        (Some(l), Some(d)) => {
            let (r, g, b) = if dark { *d } else { *l };
            (
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
                1.0,
            )
        }
        // The last part is free space: a faint track.
        _ => {
            if dark {
                (1.0, 1.0, 1.0, 0.1)
            } else {
                (0.0, 0.0, 0.0, 0.08)
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Segment {
    pub label: String,
    pub value: u64,
}

/// One whole split into parts, with a 2px gap between parts and a legend that names each part
/// with its amount, so color is never the only key.
#[derive(Clone)]
pub struct SegmentedBar {
    pub bar: gtk::DrawingArea,
    pub legend: gtk::FlowBox,
    parts: Rc<RefCell<Vec<Segment>>>,
}

impl SegmentedBar {
    pub fn new(height: i32) -> Self {
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(height);
        bar.set_hexpand(true);
        let legend = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(6)
            .row_spacing(10)
            .column_spacing(24)
            .homogeneous(true)
            .build();
        let parts: Rc<RefCell<Vec<Segment>>> = Rc::default();
        let draw_parts = parts.clone();
        bar.set_draw_func(move |_, cr, width, height| {
            let parts = draw_parts.borrow();
            let total: u64 = parts.iter().map(|p| p.value).sum();
            if total == 0 {
                return;
            }
            let dark = adw::StyleManager::default().is_dark();
            let radius = if height > 12 {
                4.0
            } else {
                f64::from(height) / 2.0
            };
            let mut x = 0.0;
            for (i, part) in parts.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let w = part.value as f64 / total as f64 * f64::from(width);
                if w < 1.0 {
                    continue;
                }
                let index = if i == parts.len() - 1 && part.label == "Free" {
                    99
                } else {
                    i
                };
                let (r, g, b, a) = series(index, dark);
                cr.set_source_rgba(r, g, b, a);
                rounded(cr, x, 0.0, (w - 2.0).max(1.0), f64::from(height), radius);
                let _ = cr.fill();
                x += w;
            }
        });
        let style = bar.clone();
        adw::StyleManager::default().connect_dark_notify(move |_| style.queue_draw());
        SegmentedBar { bar, legend, parts }
    }

    pub fn set(&self, parts: Vec<Segment>) {
        let total: u64 = parts.iter().map(|p| p.value).sum();
        while let Some(child) = self.legend.first_child() {
            self.legend.remove(&child);
        }
        for (i, part) in parts.iter().enumerate() {
            let swatch = gtk::DrawingArea::new();
            swatch.set_content_width(10);
            swatch.set_content_height(10);
            swatch.set_valign(gtk::Align::Start);
            swatch.set_margin_top(3);
            let index = if i == parts.len() - 1 && part.label == "Free" {
                99
            } else {
                i
            };
            swatch.set_draw_func(move |_, cr, w, h| {
                let (r, g, b, a) = series(index, adw::StyleManager::default().is_dark());
                cr.set_source_rgba(r, g, b, if index == 99 { a * 3.0 } else { a });
                rounded(cr, 0.0, 0.0, f64::from(w), f64::from(h), 3.0);
                let _ = cr.fill();
            });
            #[allow(clippy::cast_precision_loss)]
            let share = if total > 0 {
                part.value as f64 / total as f64 * 100.0
            } else {
                0.0
            };
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            text.append(&label(&part.label, &["dim-label", "caption"]));
            let amount = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            amount.append(&label(&bytes(part.value), &["heading", "numeric"]));
            amount.append(&label(
                &format!("{share:.0}%"),
                &["dim-label", "caption", "numeric"],
            ));
            text.append(&amount);
            let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            item.append(&swatch);
            item.append(&text);
            self.legend.insert(&item, -1);
        }
        *self.parts.borrow_mut() = parts;
        self.bar.queue_draw();
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}
