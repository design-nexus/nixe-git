use super::{diff_view::DiffView, window::current};
use gtk::{glib, prelude::*};
use nyx_git::core::{
    file_history::{self, Entry, PAGE_SIZE},
    jobs::{self, Control},
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

struct View {
    dialog: gtk::Window,
    root: PathBuf,
    file: PathBuf,
    anchor: RefCell<Option<String>>,
    offset: Cell<usize>,
    requested: Cell<usize>,
    has_older: Cell<bool>,
    entries: RefCell<Vec<Entry>>,
    rows: gtk::ListBox,
    metadata: gtk::Label,
    readout: gtk::Label,
    newer: gtk::Button,
    older: gtk::Button,
    stop: gtk::Button,
    copy: gtk::Button,
    reload: gtk::Button,
    diff: DiffView,
    page_control: RefCell<Option<Arc<Control>>>,
    diff_control: RefCell<Option<Arc<Control>>>,
    diff_generation: Cell<u64>,
    syncing: Cell<bool>,
}
pub fn open(root: PathBuf, file: PathBuf, anchor: Option<String>) {
    let (dialog, body) = widgets::dialog("File history", 740);
    dialog.set_default_height(720);
    let heading = widgets::label(&format!("{}\n{}", root.display(), file.display()), "dim-label");
    heading.set_wrap(true);
    heading.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    heading.set_selectable(true);
    body.append(&heading);
    let controls = widgets::hbox(6);
    let newer = gtk::Button::with_label("Newer");
    let older = gtk::Button::with_label("Older");
    let stop = gtk::Button::with_label("Stop loading");
    let copy = gtk::Button::with_label("Copy commit hash");
    let reload = gtk::Button::with_label("Reload page");
    for b in [&newer, &older, &stop, &copy] {
        controls.append(b);
    }
    controls.append(&reload);
    body.append(&controls);
    let readout = widgets::label("Loading file history…", "dim-label");
    readout.set_wrap(true);
    body.append(&readout);
    let rows = gtk::ListBox::new();
    rows.add_css_class("git-files");
    rows.set_selection_mode(gtk::SelectionMode::Single);
    rows.update_property(&[gtk::accessible::Property::Label("Commits affecting this file")]);
    body.append(
        &gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(140)
            .max_content_height(220)
            .propagate_natural_height(true)
            .child(&rows)
            .build(),
    );
    let metadata = widgets::label("", "dim-label");
    metadata.set_wrap(true);
    metadata.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    metadata.set_selectable(true);
    metadata.set_lines(5);
    metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);
    body.append(&metadata);
    let diff = DiffView::new();
    body.append(&diff.root);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    body.append(&close);
    let view = Rc::new(View {
        dialog,
        root,
        file,
        anchor: RefCell::new(anchor),
        offset: Cell::new(0),
        requested: Cell::new(0),
        has_older: Cell::new(false),
        entries: RefCell::new(Vec::new()),
        rows,
        metadata,
        readout,
        newer,
        older,
        stop,
        copy,
        reload,
        diff,
        page_control: RefCell::new(None),
        diff_control: RefCell::new(None),
        diff_generation: Cell::new(0),
        syncing: Cell::new(false),
    });
    let weak = Rc::downgrade(&view);
    view.rows.connect_row_selected(move |_, row| {
        if let Some(view) = weak.upgrade()
            && !view.syncing.get()
        {
            view.select(row.map(|r| r.index() as usize));
        }
    });
    let weak = Rc::downgrade(&view);
    view.newer.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade() {
            let offset = view.offset.get().saturating_sub(PAGE_SIZE);
            view.load(offset);
        }
    });
    let weak = Rc::downgrade(&view);
    view.older.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade() {
            let offset = view.offset.get().saturating_add(PAGE_SIZE);
            view.load(offset);
        }
    });
    let weak = Rc::downgrade(&view);
    view.reload.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade() {
            let offset = view.requested.get();
            view.load(offset);
        }
    });
    let weak = Rc::downgrade(&view);
    view.stop.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade() {
            view.cancel();
        }
    });
    let weak = Rc::downgrade(&view);
    view.copy.connect_clicked(move |_| {
        if let Some(view) = weak.upgrade()
            && let Some(row) = view.rows.selected_row()
            && let Some(entry) = view.entries.borrow().get(row.index() as usize)
        {
            gtk::prelude::WidgetExt::display(&view.dialog).clipboard().set_text(&entry.id);
        }
    });
    let weak = Rc::downgrade(&view);
    view.dialog.connect_close_request(move |_| {
        if let Some(view) = weak.upgrade() {
            view.cancel();
        }
        glib::Propagation::Proceed
    });
    view.dialog.present();
    view.load(0);
    // Retain the dialog state while open, and follow live theme changes.
    let weak = Rc::downgrade(&view);
    nyx_theme::subscribe(&view.dialog, move || {
        if let Some(view) = weak.upgrade() {
            view.diff.recolor();
        }
    });
    let keep = view.clone();
    view.dialog.connect_hide(move |_| keep.cancel());
}
impl View {
    fn cancel(&self) {
        for control in [&self.page_control, &self.diff_control] {
            if let Some(c) = control.borrow().as_ref() {
                c.cancel();
            }
        }
    }
    fn load(self: &Rc<Self>, offset: usize) {
        self.cancel();
        self.requested.set(offset);
        self.reload.set_sensitive(false);
        self.newer.set_sensitive(false);
        self.older.set_sensitive(false);
        self.stop.set_sensitive(true);
        self.readout.set_text("Loading file history…");
        let control = Control::new(true);
        *self.page_control.borrow_mut() = Some(control.clone());
        let (root, file, anchor) = (self.root.clone(), self.file.clone(), self.anchor.borrow().clone());
        let weak = Rc::downgrade(self);
        nyx_core::cmd::background(
            move || jobs::controlled(control, || file_history::page(&root, &file, anchor.as_deref(), offset, PAGE_SIZE)),
            move |result| {
                let Some(view) = weak.upgrade().filter(|v| v.dialog.is_visible()) else { return };
                view.page_control.borrow_mut().take();
                view.reload.set_sensitive(true);
                view.stop.set_sensitive(false);
                match result {
                    Ok(page) => {
                        *view.anchor.borrow_mut() = page.anchor;
                        view.offset.set(offset);
                        view.has_older.set(page.older);
                        view.syncing.set(true);
                        while let Some(child) = view.rows.first_child() {
                            view.rows.remove(&child);
                        }
                        for entry in &page.entries {
                            let subject = entry.message.lines().next().unwrap_or("").chars().take(220).collect::<String>();
                            let label = widgets::label(&format!("{}  {}\n{}", &entry.id[..10], entry.author, subject), "");
                            label.set_wrap(true);
                            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                            label.set_margin_top(5);
                            label.set_margin_bottom(5);
                            view.rows.append(&label);
                        }
                        let count = page.entries.len();
                        *view.entries.borrow_mut() = page.entries;
                        view.syncing.set(false);
                        view.newer.set_sensitive(offset > 0);
                        view.older.set_sensitive(page.older);
                        view.readout.set_text(&if count == 0 {
                            "No committed history for this path at the selected revision.".into()
                        } else {
                            format!(
                                "Commits {}–{} · rename following · diffs against the first parent",
                                offset + 1,
                                offset + count
                            )
                        });
                        if let Some(row) = view.rows.row_at_index(0) {
                            view.syncing.set(true);
                            view.rows.select_row(Some(&row));
                            view.syncing.set(false);
                            view.select(Some(0));
                        } else {
                            view.select(None);
                        }
                    }
                    Err(e) => {
                        view.readout.set_text(&format!("{e:#}"));
                        view.newer.set_sensitive(view.offset.get() > 0);
                        view.older.set_sensitive(view.has_older.get());
                    }
                }
            },
        );
    }
    fn select(self: &Rc<Self>, index: Option<usize>) {
        if let Some(c) = self.diff_control.borrow_mut().take() {
            c.cancel();
        }
        self.diff_generation.set(self.diff_generation.get() + 1);
        let generation = self.diff_generation.get();
        let entry = index.and_then(|i| self.entries.borrow().get(i).cloned());
        self.copy.set_sensitive(entry.is_some());
        let Some(entry) = entry else {
            self.metadata.set_text("");
            self.diff.set_text("");
            return;
        };
        let date = glib::DateTime::from_unix_local(entry.timestamp)
            .ok()
            .and_then(|t| t.format("%Y-%m-%d %H:%M %Z").ok())
            .map(|t| t.to_string())
            .unwrap_or_default();
        let metadata = format!(
            "{}\n{} · {}\n{}{}\n{}",
            entry.id,
            entry.author,
            date,
            entry.path.display(),
            entry.previous_path.as_ref().map(|p| format!(" ← {}", p.display())).unwrap_or_default(),
            entry.message
        );
        self.metadata.set_text(&metadata);
        self.metadata.set_tooltip_text(Some(&metadata));
        self.diff.set_text("Loading historical diff…");
        self.stop.set_sensitive(true);
        let control = Control::new(true);
        *self.diff_control.borrow_mut() = Some(control.clone());
        let root = self.root.clone();
        let weak = Rc::downgrade(self);
        nyx_core::cmd::background(
            move || jobs::controlled(control, || file_history::diff(&root, &entry)),
            move |result| {
                let Some(view) = weak.upgrade().filter(|v| v.dialog.is_visible() && v.diff_generation.get() == generation) else {
                    return;
                };
                view.diff_control.borrow_mut().take();
                view.stop.set_sensitive(view.page_control.borrow().is_some());
                view.diff.set_text(&match result {
                    Ok(bytes) if bytes.len() > 2 * 1024 * 1024 => "Diff exceeds 2 MiB. Inspect it in an external tool.".into(),
                    Ok(bytes) if bytes.is_empty() => "No content difference against the first parent.".into(),
                    Ok(bytes) => String::from_utf8(bytes)
                        .unwrap_or_else(|_| "Non-UTF-8 diff. Inspect the original bytes in an external tool.".into()),
                    Err(e) => format!("{e:#}"),
                });
            },
        );
    }
}
pub fn selected_change() {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else {
        ui.toast("Select a file first.");
        return;
    };
    open(root, file.original.unwrap_or(file.path), None);
}
