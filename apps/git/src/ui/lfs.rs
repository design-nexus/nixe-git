use super::{window::current, workflow::confirm};
use gtk::prelude::*;
use nyx_git::core::{
    lfs::{self, Action},
    operations::Operation,
};
use nyx_ui::widgets;
use std::rc::Rc;

pub fn show() {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nyx_core::cmd::background(
        move || lfs::read(&root),
        move |result| {
            let Some(ui) = current() else { return };
            let snapshot = match result {
                Ok(s) if ui.root().as_ref() == Some(&s.root) => Rc::new(s),
                Ok(_) => return,
                Err(e) => {
                    ui.error(&format!("Git LFS: {e:#}. Install git-lfs if unavailable."));
                    return;
                }
            };
            let (dialog, body) = widgets::dialog("Git LFS", 640);
            let note = widgets::label(
                "Enable LFS before staging tracked patterns. Pattern changes edit .gitattributes; stage it explicitly. Existing files need explicit restaging to convert. Untracking leaves existing LFS objects and history intact.",
                "dim-label",
            );
            note.set_wrap(true);
            note.set_max_width_chars(60);
            note.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.append(&note);
            let listing = widgets::label(
                &format!(
                    "{}\n\n{}\n\n{}",
                    snapshot.root.display(),
                    if snapshot.patterns.is_empty() { "No LFS patterns" } else { &snapshot.patterns },
                    if snapshot.files.is_empty() { "No LFS files at the current revision" } else { &snapshot.files }
                ),
                "dim-label",
            );
            listing.set_selectable(true);
            listing.set_wrap(true);
            listing.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            listing.set_max_width_chars(60);
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(super::scrollbars::policy())
                .min_content_height(160)
                .max_content_height(300)
                .propagate_natural_height(true)
                .child(&listing)
                .build();
            body.append(&scroll);
            let pattern = gtk::Entry::builder().placeholder_text("Pattern, e.g. *.psd").build();
            pattern.update_property(&[gtk::accessible::Property::Label("LFS tracking pattern")]);
            body.append(&pattern);
            let actions = gtk::FlowBox::new();
            actions.set_selection_mode(gtk::SelectionMode::None);
            actions.set_max_children_per_line(3);
            actions.set_column_spacing(6);
            actions.set_row_spacing(6);
            for (label, kind) in [("Enable locally…", 0), ("Track pattern…", 1), ("Untrack pattern…", 2)] {
                let button = gtk::Button::with_label(label);
                let (s, p, d) = (snapshot.clone(), pattern.clone(), dialog.clone());
                button.connect_clicked(move |_| {
                let action = match kind { 0 => Action::Enable, 1 => Action::Track(p.text().into()), _ => Action::Untrack(p.text().into()) };
                let effect = match &action {
                    Action::Enable => "Install repository-local LFS filters and the native pre-push hook. Existing hooks are not forcibly replaced. A failure may leave partial configuration; inspect Git operations before retrying.",
                    Action::Track(_) => "Add this glob pattern to root .gitattributes. Matching files may have timestamps refreshed. File contents, index and history are not converted automatically.",
                    _ => "Remove this pattern from root .gitattributes. Existing pointers, file contents and history are not converted. Review and stage .gitattributes explicitly."
                };
                let text = format!("{}\n{}\n{effect}", s.root.display(), if kind == 0 { String::new() } else { p.text().to_string() });
                let s = s.clone();
                d.close();
                confirm(action.label(), &text, "Apply", move || execute((*s).clone(), action.clone()));
            });
                actions.insert(&button, -1);
            }
            body.append(&actions);
            let remote = gtk::DropDown::from_strings(&snapshot.remotes.iter().map(String::as_str).collect::<Vec<_>>());
            super::compact_dropdown(&remote);
            remote.update_property(&[gtk::accessible::Property::Label("LFS fetch remote")]);
            remote.set_sensitive(!snapshot.remotes.is_empty());
            body.append(&remote);
            let buttons = widgets::hbox(6);
            let fetch = gtk::Button::with_label("Fetch objects…");
            fetch.set_sensitive(!snapshot.remotes.is_empty());
            let (s, d) = (snapshot.clone(), dialog.clone());
            fetch.connect_clicked(move |_| {
            let Some(name) = s.remotes.get(remote.selected() as usize).cloned() else { return };
            let s = s.clone();
            d.close();
            confirm("Fetch LFS objects", &format!("{}\nRemote: {name}\nDownload objects for the current commit using native LFS endpoints, credentials and include/exclude filters. Working files stay in place. Stop is available in Git operations.", s.root.display()), "Fetch", move || execute((*s).clone(), Action::Fetch(name.clone())));
        });
            buttons.append(&fetch);
            let checkout = gtk::Button::with_label("Check out cached…");
            checkout.set_tooltip_text(Some("Fill missing files and matching pointers using already downloaded LFS objects"));
            let (s, d) = (snapshot.clone(), dialog.clone());
            checkout.connect_clicked(move |_| {
                let s = s.clone();
                d.close();
                confirm(
                    "Check out cached LFS objects",
                    &format!("{}\nFill missing LFS files and matching pointer files from the local object cache. Deleted LFS files may reappear. Native Git LFS preserves modified files. Nothing is downloaded or staged; unavailable objects remain missing or as pointers. Fetch objects first if needed.", s.root.display()),
                    "Check out",
                    move || execute((*s).clone(), Action::Checkout),
                );
            });
            buttons.append(&checkout);
            let refresh = gtk::Button::with_label("Refresh");
            let d = dialog.clone();
            refresh.connect_clicked(move |_| {
                d.close();
                show();
            });
            buttons.append(&refresh);
            let close = gtk::Button::with_label("Close");
            let d = dialog.clone();
            close.connect_clicked(move |_| d.close());
            buttons.append(&close);
            body.append(&buttons);
            dialog.present();
        },
    );
}
fn execute(snapshot: lfs::Snapshot, action: Action) {
    if let Some(ui) = current() {
        let root = snapshot.root.clone();
        ui.execute_at_then(root.clone(), Operation::Lfs { action, expected: Box::new(snapshot) }, move |_| {
            if current().and_then(|ui| ui.root()).as_ref() == Some(&root) {
                show();
            }
        });
    }
}
