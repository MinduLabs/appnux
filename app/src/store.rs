//! What every page reads, and a tiny event bus so pages refresh when a scan lands.
//! Main thread only: scans run on workers and hand results back to the main loop.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use appnux_core::cleaner::CleanGroup;
use appnux_core::disk::DiskOverview;
use appnux_core::{App, AppFootprint, ProcessUsage, SystemInfo};

use crate::prefs::Prefs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    System,
    Apps,
    Footprints,
    Usage,
    Disk,
    ScanStarted,
    Scanned,
    Selection,
    Activity,
    Prefs,
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub time: u64,
    pub title: String,
    pub freed: u64,
    pub steps: Vec<appnux_core::actions::StepResult>,
}

impl Activity {
    pub fn failed(&self) -> usize {
        self.steps.iter().filter(|s| !s.ok).count()
    }
}

#[derive(Default)]
pub struct State {
    pub system: Option<SystemInfo>,
    pub apps: Vec<App>,
    pub apps_loading: bool,
    pub footprints: HashMap<String, AppFootprint>,
    pub footprints_loading: bool,
    pub usage: HashMap<String, ProcessUsage>,
    pub disk: Option<DiskOverview>,
    pub groups: Option<Vec<CleanGroup>>,
    pub scanning: bool,
    /// Ticked cleaner items, by id.
    pub selected: HashSet<String>,
    pub activity: Vec<Activity>,
    pub prefs: Prefs,
}

impl State {
    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// Package plus everything matched to it in the home folder.
    pub fn total(&self, app: &App) -> u64 {
        self.footprints.get(&app.id).map_or(app.install_size, |f| {
            app.install_size + f.config + f.data + f.cache
        })
    }

    /// Newest of: running now, GNOME's last-seen, the program's access time, its folders' changes.
    pub fn last_used(&self, app: &App) -> Option<u64> {
        if self.usage.contains_key(&app.id) {
            return Some(now());
        }
        let folders = self.footprints.get(&app.id).and_then(|f| f.last_activity);
        app.last_used.max(folders)
    }

    pub fn picked(&self) -> Vec<appnux_core::cleaner::CleanItem> {
        self.groups
            .iter()
            .flatten()
            .flat_map(|g| g.items.iter())
            .filter(|i| self.selected.contains(&i.id))
            .cloned()
            .collect()
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

type Listener = Rc<dyn Fn(Event)>;

#[derive(Default)]
pub struct Store {
    pub state: RefCell<State>,
    listeners: RefCell<Vec<Listener>>,
}

pub type Shared = Rc<Store>;

impl Store {
    pub fn on(&self, listener: impl Fn(Event) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Listeners run after every borrow is released, so they may read or change the state.
    pub fn emit(&self, event: Event) {
        let listeners: Vec<Listener> = self.listeners.borrow().clone();
        for listener in listeners {
            listener(event);
        }
    }

    pub fn update(&self, event: Event, change: impl FnOnce(&mut State)) {
        change(&mut self.state.borrow_mut());
        self.emit(event);
    }
}
