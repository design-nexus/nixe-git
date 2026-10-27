use super::{window::current, workflow::confirm};
use gtk::prelude::*;
use nyx_git::core::{
    operations::Operation,
    remotes,
    submodules::{self, Action, State},
};
use nyx_ui::widgets;
use std::rc::Rc;
fn display_url(value: &str) -> String {
    if value.starts_with("../") || value.starts_with("./") { value.into() } else { remotes::display_url(value) }
}
pub fn show() {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nyx_core::cmd::background(
        move || submodules::read(&root),
        move |result| {
            let Some(ui) = current() else { return };
            let snapshot = match result {
                Ok(s) if ui.root().as_ref() == Some(&s.root) => Rc::new(s),
                Ok(_) => return,
                Err(e) => {
                    ui.error(&format!("{e:#}"));
                    return;
                }
            };
            let (dialog, body) = widgets::dialog("Submodules", 640);
            let note = widgets::label(
                &format!(
                    "{}\nDirect submodules only. Updates check out the commit recorded in the parent index; nested modules are managed by opening their repository.",
                    snapshot.root.display()
                ),
                "dim-label",
            );
            note.set_wrap(true);
            note.set_max_width_chars(60);
            note.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            body.append(&note);
            let names = snapshot.modules.iter().map(|m| m.path.to_string_lossy().into_owned()).collect::<Vec<_>>();
            let choice = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
            super::compact_dropdown(&choice);
            choice.set_enable_search(true);
            choice.update_property(&[gtk::accessible::Property::Label("Submodule")]);
            choice.set_sensitive(!names.is_empty());
            body.append(&choice);
            let details = widgets::label("", "dim-label");
            details.set_wrap(true);
            details.set_max_width_chars(60);
            details.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            details.set_selectable(true);
            body.append(&details);
            let actions = gtk::FlowBox::new();
            actions.set_selection_mode(gtk::SelectionMode::None);
            actions.set_max_children_per_line(2);
            actions.set_column_spacing(6);
            actions.set_row_spacing(6);
            let open = gtk::Button::with_label("Open submodule");
            let update = gtk::Button::with_label("Initialize/update…");
            let sync = gtk::Button::with_label("Sync URL…");
            for b in [&open, &update, &sync] {
                actions.insert(b, -1);
            }
            body.append(&actions);
            let refresh = {
                let (s, details, open, update, sync) =
                    (snapshot.clone(), details.clone(), open.clone(), update.clone(), sync.clone());
                move |c: &gtk::DropDown| {
                    let module = s.modules.get(c.selected() as usize);
                    if let Some(m) = module {
                        let state = match &m.state {
                            State::Uninitialized => "Uninitialized".into(),
                            State::Clean => "Clean".into(),
                            State::Dirty(n) => format!("{n} local changes"),
                            State::Conflict => "Conflicted gitlink · resolve in Git".into(),
                            State::Unavailable(e) => format!("Unavailable: {e}"),
                        };
                        details.set_text(&format!(
                            "{}\n{}\nIndexed commit: {}\nChecked out: {}\nBranch: {}\nDeclared URL: {}",
                            m.path.display(),
                            state,
                            m.expected.as_deref().unwrap_or("Unmerged"),
                            m.actual.as_deref().unwrap_or("Not initialized"),
                            m.branch.as_deref().unwrap_or("—"),
                            m.url.as_deref().map(display_url).unwrap_or_else(|| "Missing".into())
                        ));
                        open.set_sensitive(m.actual.is_some());
                        update.set_sensitive(matches!(m.state, State::Uninitialized | State::Clean));
                        sync.set_sensitive(!matches!(m.state, State::Conflict | State::Unavailable(_)));
                    } else {
                        details.set_text("No indexed submodules. Add or remove submodule declarations with Git.");
                        open.set_sensitive(false);
                        update.set_sensitive(false);
                        sync.set_sensitive(false);
                    }
                }
            };
            refresh(&choice);
            choice.connect_selected_notify(refresh);
            let (s, c, d) = (snapshot.clone(), choice.clone(), dialog.clone());
            open.connect_clicked(move |_| {
                let snapshot = (*s).clone();
                let index = c.selected() as usize;
                let d = d.clone();
                nyx_core::cmd::background(
                    move || submodules::open_path(&snapshot, index).map(|path| (snapshot.root, path)),
                    move |result| match result {
                        Ok((root, path)) => {
                            if let Some(ui) = current()
                                && ui.root().as_ref() == Some(&root)
                            {
                                d.close();
                                ui.open(path);
                            }
                        }
                        Err(e) => {
                            if let Some(ui) = current() {
                                ui.error(&format!("{e:#}"));
                            }
                        }
                    },
                );
            });
            for (button, action) in [(update, Action::Update), (sync, Action::Sync)] {
                let (s, c, d) = (snapshot.clone(), choice.clone(), dialog.clone());
                button.connect_clicked(move |_|{
                let index=c.selected()as usize;let Some(module)=s.modules.get(index)else{return};let snapshot=s.clone();d.close();
                let effect=match action{Action::Update=>"Initialize if needed, fetch missing objects using native credentials, and check out the parent index’s commit. This may detach HEAD. Local changes or ignored files block updates. A previous initialized HEAD is saved in that submodule’s Recovery before moving it. Nested submodules are not updated.",Action::Sync=>"Copy the URL declared in .gitmodules into local submodule configuration, including an initialized child’s remote where Git applies it. Working files and commits stay in place. No network request is made."};
                confirm(action.label(),&format!("{}\n{}\n{effect}",snapshot.root.display(),module.path.display()),match action{Action::Update=>"Initialize/update",Action::Sync=>"Sync URL"},move||{if let Some(ui)=current(){let root=snapshot.root.clone();ui.execute_at_then(root.clone(),Operation::Submodule{action,index,expected:Box::new((*snapshot).clone())},move |_|{if current().and_then(|ui|ui.root()).as_ref()==Some(&root){show();}});}});
            });
            }
            let buttons = widgets::hbox(6);
            buttons.set_halign(gtk::Align::End);
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
