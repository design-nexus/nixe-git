use super::window::{Ui, current};
use gtk::{gio, prelude::*};
use nixe_git::core::repository::Snapshot;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Monitor {
    root: RefCell<Option<PathBuf>>,
    paths: RefCell<Vec<PathBuf>>,
    handles: RefCell<Vec<gio::FileMonitor>>,
    dirty: Cell<bool>,
    graph: Cell<bool>,
    /// A directory appeared or went: the next refresh builds a new watch plan.
    structural: Cell<bool>,
    last: Cell<Option<Instant>>,
    fallback: Cell<bool>,
    failed: Cell<bool>,
    checked: Cell<Option<Instant>>,
}
impl Monitor {
    pub fn install(&self, snapshot: &Snapshot) {
        let root = snapshot.repository.root.clone();
        if self.root.borrow().as_ref() == Some(&root) && *self.paths.borrow() == snapshot.watch.directories {
            self.fallback.set(snapshot.watch.fallback || self.failed.get());
            return;
        }
        self.failed.set(false);
        self.structural.set(false);
        self.dirty.set(false);
        self.graph.set(false);
        for h in self.handles.borrow_mut().drain(..) {
            h.cancel();
        }
        *self.root.borrow_mut() = Some(root.clone());
        *self.paths.borrow_mut() = snapshot.watch.directories.clone();
        self.fallback.set(snapshot.watch.fallback);
        self.checked.set(Some(Instant::now()));
        for path in &snapshot.watch.directories {
            match gio::File::for_path(path).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
                Ok(handle) => {
                    handle.set_rate_limit(200);
                    let r = root.clone();
                    let git = snapshot.repository.git_dir.clone();
                    let common = snapshot.repository.common_dir.clone();
                    handle.connect_changed(move |_, file, other, event| {
                        let Some(ui) = current() else { return };
                        if ui.root().as_ref() != Some(&r) {
                            return;
                        }
                        let paths = [file.path(), other.and_then(|f| f.path())];
                        if paths.iter().flatten().all(|p| p.file_name().is_some_and(|n| n == "nixe-operation.lock")) {
                            return;
                        }
                        if matches!(
                            event,
                            gio::FileMonitorEvent::Deleted
                                | gio::FileMonitorEvent::MovedOut
                                | gio::FileMonitorEvent::Unmounted
                                | gio::FileMonitorEvent::Renamed
                        ) {
                            let invalidate = paths.iter().flatten().any(|p| ui.monitor.paths.borrow().contains(p));
                            if invalidate {
                                ui.monitor.paths.borrow_mut().clear();
                                ui.monitor.structural.set(true);
                            }
                        }
                        if matches!(
                            event,
                            gio::FileMonitorEvent::Created | gio::FileMonitorEvent::MovedIn | gio::FileMonitorEvent::Renamed
                        ) && paths.iter().flatten().any(|p| p.is_dir())
                        {
                            ui.monitor.structural.set(true);
                        }
                        let graph = paths.iter().flatten().any(|p| {
                            [&git, &common].into_iter().any(|base| {
                                p.strip_prefix(base).ok().and_then(|p| p.components().next()).is_some_and(|c| {
                                    let name = c.as_os_str().to_string_lossy();
                                    matches!(name.as_ref(), "refs" | "HEAD" | "packed-refs" | "config" | "shallow" | "worktrees")
                                        || matches!(name.as_ref(), "HEAD.lock" | "packed-refs.lock")
                                })
                            })
                        });
                        ui.monitor.changed(graph);
                    });
                    self.handles.borrow_mut().push(handle);
                }
                Err(e) => {
                    self.fallback.set(true);
                    self.failed.set(true);
                    tracing::debug!(%e,"Directory monitor unavailable; using fallback refresh");
                }
            }
        }
        // Close the initial read/watch race with one debounced refresh after installing a new plan.
        self.changed(false);
    }
    pub fn clear(&self) {
        for h in self.handles.borrow_mut().drain(..) {
            h.cancel();
        }
        self.root.borrow_mut().take();
        self.paths.borrow_mut().clear();
        self.dirty.set(false);
        self.graph.set(false);
        self.fallback.set(false);
    }
    pub fn changed(&self, graph: bool) {
        self.dirty.set(true);
        self.graph.set(self.graph.get() || graph);
        self.last.set(Some(Instant::now()));
    }
    pub fn tick(&self, ui: &std::rc::Rc<Ui>) {
        if self.fallback.get() && self.checked.get().is_none_or(|t| t.elapsed() >= Duration::from_secs(30)) {
            self.changed(true);
            self.checked.set(Some(Instant::now()));
        }
        if !self.dirty.get() || self.last.get().is_some_and(|t| t.elapsed() < Duration::from_millis(650)) || ui.busy.get() {
            return;
        }
        if ui.snapshot.borrow().as_ref().is_some_and(|s| ui.tasks.busy(&s.repository.common_dir)) {
            return;
        }
        let Some(root) = ui.root() else { return };
        if self.root.borrow().as_ref() != Some(&root) {
            return;
        }
        self.dirty.set(false);
        self.checked.set(Some(Instant::now()));
        if self.graph.replace(false) {
            ui.graph.invalidate();
            ui.sidebar.invalidate();
        }
        // Keep the watch plan unless a directory appeared or went; building one lists every file.
        let plan = if self.structural.replace(false) || self.paths.borrow().is_empty() {
            None
        } else {
            ui.snapshot.borrow().as_ref().map(|s| s.watch.clone())
        };
        ui.reload(root, plan);
    }
}
