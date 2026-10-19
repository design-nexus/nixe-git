use super::{diff_view::DiffView, window::current, workflow::confirm};
use gtk::prelude::*;
use nixe_git::core::{
    conflicts::{self, Conflict, Resolution},
    operations::Operation,
};
use nixe_ui::widgets;
use std::{path::PathBuf, rc::Rc};
fn error(message: &str) {
    if let Some(ui) = current() {
        ui.toast(message);
    }
}
pub fn choose() {
    let Some(ui) = current() else { return };
    let files = ui
        .snapshot
        .borrow()
        .as_ref()
        .map(|s| s.status.files.iter().filter(|f| f.conflicted).map(|f| f.path.clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    if files.is_empty() {
        error("No conflicted files. Refresh if Git changed externally.");
        return;
    }
    let (dialog, body) = widgets::dialog("Resolve conflicts", 560);
    for file in files {
        let b = gtk::Button::with_label(&file.to_string_lossy());
        let d = dialog.clone();
        b.connect_clicked(move |_| {
            d.close();
            open(file.clone());
        });
        body.append(&b);
    }
    dialog.present();
}
pub fn open(file: PathBuf) {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nixe_core::cmd::background(
        move || conflicts::load(&root, &file),
        move |result| match result {
            Ok(conflict) => editor(conflict),
            Err(e) => {
                if let Some(ui) = current() {
                    ui.error(&format!("{e:#}"));
                }
            }
        },
    );
}
fn submit(conflict: Conflict, resolution: Resolution, dialog: &gtk::Window) {
    if let Some(ui) = current() {
        let d = dialog.clone();
        ui.execute_at_then(conflict.root.clone(),Operation::ResolveConflict{conflict:Box::new(conflict),resolution},move|result|match result{
            Ok(())=>d.close(),
            Err(message)=>{let (notice,body)=widgets::dialog("Resolution could not be completed",560);notice.set_transient_for(Some(&d));
                let label=widgets::label(&format!("{message}\nYour editor draft remains open. If the repository changed, reopen this conflict before retrying."),"dim-label");label.set_wrap(true);body.append(&label);
                let close=gtk::Button::with_label("Close");let n=notice.clone();close.connect_clicked(move |_|n.close());body.append(&close);notice.present();}
        });
    }
}
fn editor(conflict: Conflict) {
    let (dialog, body) = widgets::dialog("Resolve conflict", 800);
    let note = widgets::label(
        &format!(
            "{}\nOurs = index stage 2; theirs = stage 3. During rebase, ours is the branch being rebased onto, and theirs is the commit being replayed. Saving stages this file; it does not continue the operation.",
            conflict.file.display()
        ),
        "dim-label",
    );
    note.set_wrap(true);
    body.append(&note);
    if !conflict.text() {
        let note = widgets::label(
            "Binary conflict: preview and manual editing are unavailable. Choose a complete side or delete the file explicitly.",
            "dim-label",
        );
        note.set_wrap(true);
        body.append(&note);
        for (title, resolution) in
            [("Use ours…", Resolution::Ours), ("Use theirs…", Resolution::Theirs), ("Delete file…", Resolution::Delete)]
        {
            let b = gtk::Button::with_label(title);
            let c = conflict.clone();
            let d = dialog.clone();
            b.connect_clicked(move |_|{let c=c.clone();let r=resolution.clone();let d=d.clone();confirm("Resolve binary conflict?","Replace the working file with the chosen index stage, or remove it if that side is absent. The result is staged immediately. Current working contents are overwritten.",title,move||submit(c.clone(),r.clone(),&d));});
            body.append(&b);
        }
        dialog.present();
        return;
    }
    let result = Rc::new(DiffView::new());
    let view = result.root.child().unwrap().downcast::<sourceview5::View>().unwrap();
    view.set_editable(true);
    result.set_text(conflict.working.as_ref().and_then(|b| std::str::from_utf8(b).ok()).unwrap_or_default());
    let source = Rc::new(DiffView::new());
    let choices = vec![conflict.base.clone(), conflict.ours.clone(), conflict.theirs.clone()];
    let picker = gtk::DropDown::from_strings(&["Base (stage 1)", "Ours (stage 2)", "Theirs (stage 3)"]);
    picker.set_selected(1);
    source.set_text(
        choices[1].as_ref().map(|s| std::str::from_utf8(&s.bytes).unwrap_or_default()).unwrap_or("[This side has no file]"),
    );
    let s = source.clone();
    let sides = choices.clone();
    picker.connect_selected_notify(move |p| {
        s.set_text(
            sides
                .get(p.selected() as usize)
                .and_then(|s| s.as_ref())
                .map(|s| std::str::from_utf8(&s.bytes).unwrap_or_default())
                .unwrap_or("[This side has no file]"),
        );
    });
    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    left.append(&picker);
    left.append(&source.root);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 6);
    right.append(&widgets::label("EDITABLE RESULT", "dim-label"));
    right.append(&result.root);
    let panes = gtk::Paned::new(gtk::Orientation::Horizontal);
    panes.set_start_child(Some(&left));
    panes.set_end_child(Some(&right));
    panes.set_position(370);
    panes.set_resize_start_child(true);
    panes.set_resize_end_child(true);
    panes.set_shrink_start_child(true);
    panes.set_shrink_end_child(true);
    panes.set_vexpand(true);
    panes.set_size_request(-1, 320);
    body.append(&panes);
    let actions = gtk::FlowBox::new();
    actions.set_selection_mode(gtk::SelectionMode::None);
    actions.set_max_children_per_line(3);
    actions.set_column_spacing(6);
    actions.set_row_spacing(6);
    let whole = gtk::Button::with_label("Use selected side");
    let r = result.clone();
    let sides = choices.clone();
    let p = picker.clone();
    whole.connect_clicked(move |_| {
        if let Some(Some(side)) = sides.get(p.selected() as usize) {
            r.set_text(std::str::from_utf8(&side.bytes).unwrap_or_default());
        } else {
            error("This side has no file. Use Delete file to resolve as a deletion.");
        }
    });
    actions.insert(&whole, -1);
    let selected = gtk::Button::with_label("Insert selected text");
    selected.set_tooltip_text(Some("Copy selected source text over the result selection, or insert it at the result cursor"));
    let s = source.clone();
    let r = result.clone();
    selected.connect_clicked(move |_| {
        let Some((start, end)) = s.buffer.selection_bounds() else {
            error("Select text in the source pane first.");
            return;
        };
        let text = s.buffer.text(&start, &end, false);
        if let Some((mut start, mut end)) = r.buffer.selection_bounds() {
            r.buffer.delete(&mut start, &mut end);
            r.buffer.insert(&mut start, &text);
        } else {
            r.buffer.insert_at_cursor(&text);
        }
    });
    actions.insert(&selected, -1);
    let next = gtk::Button::with_label("Next conflict marker");
    let r = result.clone();
    next.connect_clicked(move |_| {
        let text = r.buffer.text(&r.buffer.start_iter(), &r.buffer.end_iter(), false);
        let current = r.buffer.iter_at_offset(r.buffer.cursor_position()).line();
        let lines =
            text.lines().enumerate().filter(|(_, l)| conflicts::has_markers(l)).map(|(i, _)| i as i32).collect::<Vec<_>>();
        if let Some(line) = lines.iter().find(|i| **i > current).or(lines.first()) {
            if let Some(iter) = r.buffer.iter_at_line(*line) {
                r.buffer.place_cursor(&iter);
                if let Some(view) = r.root.child().and_downcast::<sourceview5::View>() {
                    let mut iter = iter;
                    view.scroll_to_iter(&mut iter, 0.1, false, 0., 0.);
                    view.grab_focus();
                }
            }
        } else {
            error("No conflict markers remain.");
        }
    });
    actions.insert(&next, -1);
    let save = gtk::Button::with_label("Save and mark resolved…");
    save.add_css_class("suggested-action");
    let c = conflict.clone();
    let d = dialog.clone();
    let r = result.clone();
    save.connect_clicked(move |_|{
        let text=r.buffer.text(&r.buffer.start_iter(),&r.buffer.end_iter(),false).to_string();if conflicts::has_markers(&text){error("Remove every conflict marker before marking this file resolved.");return;}
        let c=c.clone();let d=d.clone();confirm("Save resolution?","Overwrite the working file with the edited result and stage it as resolved. Continue the Git operation separately after resolving every file.","Save and stage",move||submit(c.clone(),Resolution::Text(text.clone()),&d));
    });
    actions.insert(&save, -1);
    let delete = gtk::Button::with_label("Delete file…");
    let c = conflict.clone();
    let d = dialog.clone();
    delete.connect_clicked(move |_| {
        let c = c.clone();
        let d = d.clone();
        confirm(
            "Resolve as deletion?",
            "Remove the working file and stage its deletion. Current working contents are lost.",
            "Delete and stage",
            move || submit(c.clone(), Resolution::Delete, &d),
        );
    });
    actions.insert(&delete, -1);
    body.append(&actions);
    // Mark unresolved delimiters with the same semantic danger color as diff deletions.
    let b = result.buffer.clone();
    result.buffer.connect_changed(move |_| {
        b.remove_tag_by_name("deletion", &b.start_iter(), &b.end_iter());
        let text = b.text(&b.start_iter(), &b.end_iter(), false);
        for (line, text) in text.lines().enumerate() {
            if conflicts::has_markers(text)
                && let Some(start) = b.iter_at_line(line as i32)
            {
                let mut end = start;
                end.forward_to_line_end();
                b.apply_tag_by_name("deletion", &start, &end);
            }
        }
    });
    let initial = result.buffer.text(&result.buffer.start_iter(), &result.buffer.end_iter(), false);
    result.buffer.set_text(&initial);
    let weak_source = Rc::downgrade(&source);
    let weak_result = Rc::downgrade(&result);
    nixe_theme::subscribe(&dialog, move || {
        if let Some(view) = weak_source.upgrade() {
            view.recolor();
        }
        if let Some(view) = weak_result.upgrade() {
            view.recolor();
        }
    });
    dialog.present();
}
