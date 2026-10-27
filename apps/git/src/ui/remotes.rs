use super::{window::current, workflow::confirm};
use gtk::prelude::*;
use nyx_git::core::{
    operations::Operation,
    remotes::{self, Action, Remote, Snapshot},
};
use nyx_ui::widgets;
use std::rc::Rc;

fn note(body: &gtk::Box, text: &str) -> gtk::Label {
    let label = widgets::label(text, "dim-label");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.append(&label);
    label
}
pub fn show() {
    show_selected(None);
}
pub fn show_selected(selected: Option<String>) {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nyx_core::cmd::background(
        move || remotes::read(&root),
        move |result| {
            let Some(ui) = current() else { return };
            let snapshot = match result {
                Ok(snapshot) if ui.root().as_ref() == Some(&snapshot.root) => Rc::new(snapshot),
                Ok(_) => return,
                Err(e) => {
                    ui.error(&format!("{e:#}"));
                    return;
                }
            };
            let (dialog, body) = widgets::dialog("Remotes", 600);
            note(
                &body,
                "Configure fetch and push destinations locally. Server branches contacts the push destination to list or delete branches.",
            );
            let names = snapshot.remotes.iter().map(|r| r.name.as_str()).collect::<Vec<_>>();
            let choice = gtk::DropDown::from_strings(&names);
            if let Some(index) = selected.as_ref().and_then(|name| names.iter().position(|n| *n == name)) {
                choice.set_selected(index as u32);
            }
            super::compact_dropdown(&choice);
            choice.update_property(&[gtk::accessible::Property::Label("Remote")]);
            choice.set_sensitive(!names.is_empty());
            body.append(&choice);
            let details = note(&body, "");
            details.set_selectable(true);
            let actions = gtk::FlowBox::new();
            actions.set_selection_mode(gtk::SelectionMode::None);
            actions.set_max_children_per_line(3);
            actions.set_column_spacing(6);
            actions.set_row_spacing(6);
            let mut buttons = Vec::new();
            for (title, edit) in [
                ("Add…", Edit::Add),
                ("Rename…", Edit::Rename),
                ("Fetch URL…", Edit::Fetch),
                ("Push URL…", Edit::Push),
                ("Remove…", Edit::Remove),
            ] {
                let button = gtk::Button::with_label(title);
                let s = snapshot.clone();
                let c = choice.clone();
                let d = dialog.clone();
                button.connect_clicked(move |_| {
                let remote = s.remotes.get(c.selected() as usize).cloned();
                if !matches!(edit, Edit::Add) && remote.is_none() { return; }
                d.close();
                if matches!(edit, Edit::Remove) {
                    let name = remote.unwrap().name;
                    let snapshot = s.clone();
                    confirm("Remove remote?", &format!("Remove {name} from {} and delete its local remote-tracking refs and upstream configuration. Tracking tips are retained in Recovery. Local branches, working files and the server remain. No network request is made.", snapshot.root.display()), "Remove remote", move || {
                        if let Some(ui) = current() {
                            ui.execute_at_then(snapshot.root.clone(), Operation::Remote { action: Action::Remove { name: name.clone() }, expected: Box::new((*snapshot).clone()) }, |_| show());
                        }
                    });
                } else { editor(s.clone(), remote, edit); }
            });
                actions.insert(&button, -1);
                buttons.push(button);
            }
            body.append(&actions);
            let server = gtk::Button::with_label("Server branches…");
            server.set_tooltip_text(Some("Contact the push destination to list branches and delete a selected branch safely"));
            server.set_sensitive(!names.is_empty());
            let (s, c, d) = (snapshot.clone(), choice.clone(), dialog.clone());
            server.connect_clicked(move |_| {
                if let Some(remote) = s.remotes.get(c.selected() as usize) {
                    d.close();
                    server_branches(remote.name.clone());
                }
            });
            body.append(&server);
            let update = {
                let s = snapshot.clone();
                move |choice: &gtk::DropDown| {
                    let remote = s.remotes.get(choice.selected() as usize);
                    details.set_text(
                        &remote
                            .map(|r| {
                                format!(
                                    "Fetch:\n{}\n\nPush:\n{}{}",
                                    r.fetch.iter().map(|u| remotes::display_url(u)).collect::<Vec<_>>().join("\n"),
                                    if r.push.is_empty() {
                                        "Uses fetch URL".into()
                                    } else {
                                        r.push.iter().map(|u| remotes::display_url(u)).collect::<Vec<_>>().join("\n")
                                    },
                                    if !r.editable {
                                        "\n\nInherited or included configuration · manage this remote with Git."
                                    } else if !r.remove_safe {
                                        "\n\nMirror or custom fetch mappings · remove this remote with Git."
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .unwrap_or_else(|| "No remotes configured. Add a destination to fetch or push.".into()),
                    );
                    for (i, button) in buttons.iter().enumerate().skip(1) {
                        button.set_sensitive(remote.is_some_and(|r| {
                            r.editable
                                && match i {
                                    2 => r.fetch.len() == 1,
                                    3 => r.push.len() <= 1,
                                    4 => r.remove_safe,
                                    _ => true,
                                }
                        }));
                    }
                }
            };
            update(&choice);
            choice.connect_selected_notify(update);
            let close = gtk::Button::with_label("Close");
            close.set_halign(gtk::Align::End);
            let d = dialog.clone();
            close.connect_clicked(move |_| d.close());
            body.append(&close);
            dialog.present();
        },
    );
}

fn server_branches(remote: String) {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    let (dialog, body) = widgets::dialog("Server branches", 600);
    let status = note(&body, &format!("Loading branches from {remote}’s push destination…"));
    let choice = gtk::DropDown::from_strings(&[]);
    super::compact_dropdown(&choice);
    choice.set_enable_search(true);
    choice.update_property(&[gtk::accessible::Property::Label("Server branch")]);
    choice.set_sensitive(false);
    body.append(&choice);
    let details = note(&body, "");
    details.set_selectable(true);
    let delete = gtk::Button::with_label("Delete selected branch…");
    delete.add_css_class("destructive-action");
    delete.set_sensitive(false);
    body.append(&delete);
    let buttons = widgets::hbox(6);
    buttons.set_halign(gtk::Align::End);
    let refresh = gtk::Button::with_label("Refresh");
    let (d, name) = (dialog.clone(), remote.clone());
    refresh.connect_clicked(move |_| {
        d.close();
        server_branches(name.clone());
    });
    buttons.append(&refresh);
    let close = gtk::Button::with_label("Close");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    buttons.append(&close);
    body.append(&buttons);
    dialog.present();
    nyx_core::cmd::background(
        move || remotes::branches(&root, &remote),
        move |result| {
            if !dialog.is_visible() {
                return;
            }
            let listing = match result {
                Ok(listing) => Rc::new(listing),
                Err(e) => {
                    status.set_text(&format!("{e:#}"));
                    return;
                }
            };
            if current().and_then(|ui| ui.root()).as_ref() != Some(&listing.snapshot.root) {
                status.set_text("The active repository changed. Reopen server branches.");
                return;
            }
            status.set_text(&format!(
                "{} · {} branches on the push destination. The server’s default branch is protected here.",
                listing.remote,
                listing.branches.len()
            ));
            let names = listing.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>();
            choice.set_model(Some(&gtk::StringList::new(&names)));
            choice.set_sensitive(!names.is_empty());
            let update = {
                let (listing, details, delete) = (listing.clone(), details.clone(), delete.clone());
                move |choice: &gtk::DropDown| {
                    let branch = listing.branches.get(choice.selected() as usize);
                    details.set_text(
                        &branch
                            .map(|b| {
                                format!(
                                    "{}\n{}{}",
                                    b.name,
                                    b.id,
                                    if listing.default.as_deref() == Some(&b.name) {
                                        "\nDefault branch"
                                    } else if listing.default.is_none() {
                                        "\nServer did not advertise its default branch; deletion is unavailable."
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .unwrap_or_else(|| "No server branches found.".into()),
                    );
                    delete.set_sensitive(
                        branch.is_some_and(|b| listing.default.as_deref().is_some_and(|default| default != b.name)),
                    );
                }
            };
            update(&choice);
            choice.connect_selected_notify(update);
            let d = dialog.clone();
            delete.connect_clicked(move |_| {
            let Some(branch) = listing.branches.get(choice.selected() as usize).cloned() else { return };
            d.close();
            let listing = listing.clone();
            confirm("Delete server branch?", &format!("Delete {} from {}’s push destination for {} at {}. Nyx first fetches this tip into local Recovery, then deletes only if the server tip is unchanged. Local branches and working files remain. Server hooks and permissions still apply.", branch.name, listing.remote, listing.snapshot.root.display(), branch.id), "Delete server branch", move || {
                if let Some(ui) = current() {
                    ui.execute_at(listing.snapshot.root.clone(), Operation::DeleteRemoteBranch { listing: Box::new((*listing).clone()), branch: branch.name.clone(), expected: branch.id.clone() });
                }
            });
        });
        },
    );
}
#[derive(Clone, Copy)]
enum Edit {
    Add,
    Rename,
    Fetch,
    Push,
    Remove,
}
fn entry(body: &gtk::Box, label: &str, value: &str) -> gtk::Entry {
    body.append(&widgets::label(label, "dim-label"));
    let entry = gtk::Entry::new();
    entry.set_text(value);
    entry.update_property(&[gtk::accessible::Property::Label(label)]);
    body.append(&entry);
    entry
}
fn editor(snapshot: Rc<Snapshot>, remote: Option<Remote>, edit: Edit) {
    let (title, verb) = match edit {
        Edit::Add => ("Add remote", "Add remote"),
        Edit::Rename => ("Rename remote", "Rename"),
        Edit::Fetch => ("Fetch URL", "Save fetch URL"),
        Edit::Push => ("Push URL", "Save push URL"),
        Edit::Remove => return,
    };
    let (dialog, body) = widgets::dialog(title, 560);
    note(&body, &format!("Repository: {}", snapshot.root.display()));
    let name = remote.as_ref().map(|r| r.name.as_str()).unwrap_or("origin");
    let name_entry = entry(&body, "Remote name", name);
    name_entry.set_editable(matches!(edit, Edit::Add | Edit::Rename));
    let initial = match (edit, remote.as_ref()) {
        (Edit::Fetch, Some(r)) => r.fetch.first(),
        (Edit::Push, Some(r)) => r.push.first(),
        _ => None,
    }
    .filter(|u| remotes::validate_url(u).is_ok())
    .cloned()
    .unwrap_or_default();
    let url_label = widgets::label("URL", "dim-label");
    url_label.set_visible(!matches!(edit, Edit::Rename));
    body.append(&url_label);
    let url = gtk::Entry::new();
    url.set_text(&initial);
    url.update_property(&[gtk::accessible::Property::Label("URL")]);
    body.append(&url);
    url.set_visible(!matches!(edit, Edit::Rename));
    if !matches!(edit, Edit::Rename) {
        note(
            &body,
            "Use HTTPS, SSH (including git@host:path), or an absolute local path. Use Git credential helpers or SSH agents for authentication.",
        );
        if matches!(edit, Edit::Push) {
            note(&body, "Leave the push URL empty to use the fetch URL.");
        }
    }
    let status = note(&body, "");
    status.add_css_class("git-error");
    let buttons = widgets::hbox(6);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    buttons.append(&cancel);
    let save = gtk::Button::with_label(verb);
    save.add_css_class("suggested-action");
    let d = dialog.clone();
    save.connect_clicked(move |button| {
        let new = name_entry.text().trim().to_string();
        let value = url.text().trim().to_string();
        let name = remote.as_ref().map(|r| r.name.clone()).unwrap_or_default();
        let action = match edit {
            Edit::Add => Action::Add { name: new, url: value },
            Edit::Rename => Action::Rename { name, new },
            Edit::Fetch => Action::FetchUrl { name, url: value },
            Edit::Push => Action::PushUrl { name, url: (!value.is_empty()).then_some(value) },
            Edit::Remove => return,
        };
        button.set_sensitive(false);
        status.set_text("Saving remote configuration…");
        let (d, status, button) = (d.clone(), status.clone(), button.clone());
        if let Some(ui) = current() {
            ui.execute_at_then(
                snapshot.root.clone(),
                Operation::Remote { action, expected: Box::new((*snapshot).clone()) },
                move |result| match result {
                    Ok(()) => {
                        d.close();
                        show();
                    }
                    Err(e) => {
                        status.set_text(&e);
                        button.set_sensitive(true);
                    }
                },
            );
        }
    });
    buttons.append(&save);
    body.append(&buttons);
    dialog.present();
}
