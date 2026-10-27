use super::{diff_view::DiffView, window::current};
use gtk::{glib, prelude::*};
use nyx_git::core::{
    history::{self, Details},
    operations::{self, Operation},
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};
pub struct CommitDetails {
    pub root: gtk::Box,
    metadata: gtk::Label,
    message: gtk::TextView,
    files: gtk::ListBox,
    files_title: gtk::Label,
    files_scroll: gtk::ScrolledWindow,
    paths: RefCell<Vec<PathBuf>>,
    data: RefCell<Option<Details>>,
    generation: Cell<u64>,
    diff_generation: Cell<u64>,
    syncing: Cell<bool>,
    pub diff: DiffView,
}
impl CommitDetails {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("git-details");
        root.set_hexpand(true);
        root.set_vexpand(true);
        let metadata = widgets::label("Select a commit to inspect it.", "dim-label");
        metadata.set_selectable(true);
        metadata.set_wrap(true);
        metadata.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        root.append(&metadata);
        let message = gtk::TextView::new();
        message.set_editable(false);
        message.update_property(&[gtk::accessible::Property::Label("Commit message")]);
        message.set_monospace(true);
        message.set_wrap_mode(gtk::WrapMode::WordChar);
        message.add_css_class("git-message");
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(super::scrollbars::policy())
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_height(90)
            .max_content_height(130)
            .child(&message)
            .build();
        root.append(&scroll);
        let heading = widgets::hbox(8);
        let files_title = widgets::label("CHANGED FILES", "group-title");
        files_title.set_hexpand(true);
        files_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.append(&files_title);
        let history = gtk::Button::with_label("File history…");
        history.set_valign(gtk::Align::Center);
        history.set_tooltip_text(Some("The selected file's history up to this commit"));
        history.connect_clicked(|_| {
            if let Some(ui) = current()
                && let Some(root) = ui.root()
            {
                let details = &ui.graph.details;
                if let Some(file) = details.selected_path()
                    && let Some(data) = details.data.borrow().as_ref()
                {
                    super::file_history::open(root, file, Some(data.id.clone()));
                }
            }
        });
        heading.append(&history);
        root.append(&heading);
        let files = gtk::ListBox::new();
        files.add_css_class("git-commit-files");
        files.set_selection_mode(gtk::SelectionMode::Single);
        files.update_property(&[gtk::accessible::Property::Label("Files changed in selected commit")]);
        files.connect_row_selected(|_, _| {
            if let Some(ui) = current()
                && !ui.graph.details.syncing.get()
            {
                ui.graph.details.load_diff();
            }
        });
        let files_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .propagate_natural_height(true)
            .max_content_height(180)
            .child(&files)
            .build();
        root.append(&files_scroll);
        let diff = DiffView::new();
        root.append(&diff.root);
        Self {
            root,
            metadata,
            message,
            files,
            files_title,
            files_scroll,
            paths: RefCell::new(Vec::new()),
            data: RefCell::new(None),
            generation: Cell::new(0),
            diff_generation: Cell::new(0),
            syncing: Cell::new(false),
            diff,
        }
    }
    fn selected_path(&self) -> Option<PathBuf> {
        let index = self.files.selected_row()?.index();
        self.paths.borrow().get(index as usize).cloned()
    }
    pub fn focus(&self) -> bool {
        self.metadata.grab_focus()
    }
    pub fn clear(&self) {
        self.generation.set(self.generation.get() + 1);
        self.diff_generation.set(self.diff_generation.get() + 1);
        self.metadata.set_text("Select a commit to inspect it.");
        self.message.buffer().set_text("");
        *self.data.borrow_mut() = None;
        self.paths.borrow_mut().clear();
        self.diff.set_text("");
        self.files_title.set_text("CHANGED FILES");
        self.syncing.set(true);
        self.files.remove_all();
        self.syncing.set(false);
    }
    pub fn select(&self, id: String) {
        self.clear();
        let generation = self.generation.get();
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        self.metadata.set_text("Loading commit…");
        nyx_core::cmd::background(
            move || -> anyhow::Result<_> {
                let data = history::details(&root, &id)?;
                let repo = gix::discover(&root)?;
                let boundary = repo.shallow_commits()?.is_some_and(|ids| ids.iter().any(|oid| oid.to_string() == id))
                    && !data.parents.is_empty();
                let files = if boundary {
                    Vec::new()
                } else {
                    operations::run(&root, Operation::CommitFiles { id, parent: data.parents.first().cloned() })?
                };
                let records: Vec<_> = files.split(|b| *b == 0).filter(|b| !b.is_empty()).collect();
                let mut paths = Vec::new();
                let mut names = Vec::new();
                for pair in records.as_chunks::<2>().0 {
                    let path = PathBuf::from(OsString::from_vec(pair[1].to_vec()));
                    names.push(format!("{}  {}", String::from_utf8_lossy(pair[0]), path.display()));
                    paths.push(path);
                }
                Ok((root, data, paths, names, boundary))
            },
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, data, paths, names, boundary)) if ui.root().as_ref() == Some(&root) => {
                        let time = glib::DateTime::from_unix_local(data.timestamp)
                            .ok()
                            .and_then(|t| t.format("%Y-%m-%d %H:%M %Z").ok())
                            .map(|t| t.to_string())
                            .unwrap_or_default();
                        let parents = match data.parents.len() {
                            0 => "Root commit".to_string(),
                            1 => format!("Parent {}", &data.parents[0][..10]),
                            _ => format!("Merge of {}", data.parents.iter().map(|p| &p[..10]).collect::<Vec<_>>().join(" · ")),
                        };
                        me.metadata.set_text(&format!(
                            "{} <{}>\n{time}\nCommit {}\n{parents}",
                            data.author, data.email, data.id
                        ));
                        me.files_title.set_text(&format!(
                            "CHANGED FILES · {}{}",
                            paths.len(),
                            if data.parents.len() > 1 { " · vs first parent" } else { "" }
                        ));
                        me.message.buffer().set_text(&data.message);
                        *me.data.borrow_mut() = Some(data);
                        *me.paths.borrow_mut() = paths;
                        me.syncing.set(true);
                        for name in &names {
                            let label = widgets::label(name, "");
                            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                            label.set_tooltip_text(Some(name));
                            me.files.append(&label);
                        }
                        me.files.select_row(me.files.row_at_index(0).as_ref());
                        me.files_scroll.vadjustment().set_value(0.0);
                        me.syncing.set(false);
                        if boundary {
                            me.diff.set_text("Shallow history boundary: the parent object is unavailable. Fetch more history to inspect these changes.");
                        } else {
                            me.load_diff();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => me.metadata.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    fn load_diff(&self) {
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        let generation = self.diff_generation.get() + 1;
        self.diff_generation.set(generation);
        let Some(data) = self.data.borrow().clone() else { return };
        let Some(path) = self.selected_path() else {
            self.diff.set_text("No file changes in this commit.");
            return;
        };
        self.diff.set_text("Loading diff…");
        nyx_core::cmd::background(
            move || {
                operations::run(&root, Operation::CommitDiff { id: data.id, parent: data.parents.first().cloned(), file: path })
                    .map(|bytes| (root, bytes))
            },
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.diff_generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, bytes)) if ui.root().as_ref() == Some(&root) => me.show_bytes(&bytes),
                    Ok(_) => {}
                    Err(e) => me.diff.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    pub fn compare(&self, base: String, tip: String) {
        self.clear();
        let generation = self.generation.get();
        self.metadata.set_text(&format!("Compare {} → {}", &base[..10], &tip[..10]));
        self.diff.set_text("Loading comparison…");
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        nyx_core::cmd::background(
            move || operations::run(&root, Operation::Compare { base, tip }).map(|bytes| (root, bytes)),
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, bytes)) if ui.root().as_ref() == Some(&root) => me.show_bytes(&bytes),
                    Ok(_) => {}
                    Err(e) => me.diff.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    fn show_bytes(&self, bytes: &[u8]) {
        self.diff.set_text(if bytes.len() > 2 * 1024 * 1024 {
            "Diff exceeds 2 MiB. Inspect it with an external tool."
        } else if bytes.is_empty() {
            "No content differences."
        } else {
            std::str::from_utf8(bytes).unwrap_or("Non-UTF-8 diff. Inspect the original bytes with an external tool.")
        });
    }
}
