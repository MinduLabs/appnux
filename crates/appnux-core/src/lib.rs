//! Appnux core. Everything that reads the system lives here; the Tauri shell only forwards.
//!
//! Nothing in this crate deletes anything yet: plans are built and previewed, and execution is a
//! dry run until the privileged helper and the safety layer land (docs/PLAN.md, Phase 1).

pub mod actions;
pub mod apps;
pub mod cleaner;
pub mod desktop;
pub mod disk;
pub mod distro;
pub mod model;
pub mod plan;
pub mod procs;
pub mod residue;
mod run;

pub use model::*;

/// Scans run on a small pool: a desktop utility should not take every core, and each worker
/// costs memory.
pub fn limit_threads() {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    if let Err(err) = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
    {
        tracing::warn!(%err, "scan pool already set up");
    }
}
