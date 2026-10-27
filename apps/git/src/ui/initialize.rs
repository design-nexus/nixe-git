use super::window::current;
use gtk::{gio, prelude::*};
use nyx_git::core::{
    jobs,
    operations::{self, Operation},
};
use nyx_ui::widgets;
use std::path::PathBuf;

pub fn show() {
    let (dialog, body) = widgets::dialog("Initialize repository", 560);
    let info = widgets::label(
        "Create an empty Git repository in a new or existing directory. Existing files stay untracked. Choose a location outside another repository.",
        "dim-label",
    );
    info.set_wrap(true);
    body.append(&info);
    body.append(&widgets::label("Repository directory", "dim-label"));
    let path = gtk::Entry::new();
    path.set_placeholder_text(Some("Absolute directory path"));
    path.update_property(&[gtk::accessible::Property::Label("Repository directory")]);
    body.append(&path);
    let browse = gtk::Button::with_label("Choose existing directory…");
    let (d, p) = (dialog.clone(), path.clone());
    browse.connect_clicked(move |_| {
        let p = p.clone();
        gtk::FileDialog::builder().title("Repository directory").build().select_folder(
            Some(&d),
            gio::Cancellable::NONE,
            move |result| match result {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        p.set_text(&path.to_string_lossy());
                    }
                }
                Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => {}
                Err(e) => {
                    if let Some(ui) = current() {
                        ui.error(&format!("Could not choose a directory: {e}"));
                    }
                }
            },
        );
    });
    body.append(&browse);
    body.append(&widgets::label("Initial branch", "dim-label"));
    let branch = gtk::Entry::new();
    branch.set_text("main");
    branch.update_property(&[gtk::accessible::Property::Label("Initial branch")]);
    body.append(&branch);
    let status = widgets::label("", "git-error");
    status.set_wrap(true);
    body.append(&status);
    let actions = widgets::hbox(6);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    actions.append(&cancel);
    let create = gtk::Button::with_label("Initialize and open");
    create.add_css_class("suggested-action");
    let d = dialog.clone();
    create.connect_clicked(move |button| {
        let Some(ui) = current() else { return };
        let destination = PathBuf::from(path.text().trim());
        if !destination.is_absolute() {
            status.set_text("Choose an absolute repository directory.");
            return;
        }
        let operation = Operation::Initialize { destination: destination.clone(), branch: branch.text().trim().into() };
        let (id, control) = match ui.tasks.start(destination.clone(), destination.clone(), &operation) {
            Ok(task) => task,
            Err(e) => {
                status.set_text(&e.to_string());
                return;
            }
        };
        ui.update_tasks();
        button.set_sensitive(false);
        status.set_text("Initializing repository…");
        let (d, status, button) = (d.clone(), status.clone(), button.clone());
        let weak = std::rc::Rc::downgrade(&ui);
        nyx_core::cmd::background(
            move || jobs::controlled(control, || operations::run(&destination, operation)).map(|_| destination),
            move |result| {
                let Some(ui) = weak.upgrade() else { return };
                ui.task_finished(id, result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                match result {
                    Ok(root) => {
                        d.close();
                        ui.open(root);
                    }
                    Err(e) => {
                        status.set_text(&format!("{e:#}"));
                        button.set_sensitive(true);
                    }
                }
            },
        );
    });
    actions.append(&create);
    body.append(&actions);
    dialog.present();
}
