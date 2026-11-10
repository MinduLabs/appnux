//! Runs a confirmed plan on a worker thread, records it in Activity, tells the user, and
//! refreshes what changed on disk.

use appnux_core::actions::{self, Op, StepResult};
use gtk::{gio, glib};

use crate::store::{Activity, Event, Shared, now};
use crate::ui::bytes;

pub fn run(
    store: &Shared,
    title: String,
    ops: Vec<(Op, u64)>,
    toasts: &adw::ToastOverlay,
    done: impl FnOnce(&[StepResult]) + 'static,
) {
    let store = store.clone();
    let toasts = toasts.clone();
    glib::spawn_future_local(async move {
        let results = gio::spawn_blocking(move || actions::execute(ops))
            .await
            .unwrap_or_default();
        let freed: u64 = results.iter().map(|r| r.freed).sum();
        let failed = results.iter().filter(|r| !r.ok).count();
        store.update(Event::Activity, |s| {
            s.activity.insert(
                0,
                Activity {
                    time: now(),
                    title: title.clone(),
                    freed,
                    steps: results.clone(),
                },
            );
        });
        let message = match failed {
            0 => format!("{title}: done, {} freed.", bytes(freed)),
            n if n == results.len() => {
                let why = results
                    .first()
                    .and_then(|r| r.error.clone())
                    .unwrap_or_default();
                format!("{title} didn't run: {why}")
            }
            n => format!(
                "{title}: {} freed, {n} step(s) failed. See Activity.",
                bytes(freed)
            ),
        };
        toasts.add_toast(adw::Toast::new(&message));
        done(&results);
        // What changed on disk: the app list, the cleaner, the disk bar.
        crate::reload(&store);
    });
}
