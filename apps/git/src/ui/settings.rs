use super::window::{SHORTCUTS, current};
use crate::prefs;
use gtk::prelude::*;
use nyx_ui::{settings_card, widgets};

pub fn open() {
    let Some(ui) = current() else { return };
    settings_card::open(
        &ui.overlay,
        &ui.window,
        settings_card::Spec {
            groups: &[
                ("Git window", "preferences-desktop-display-symbolic", "Theme, accent, motion, startup"),
                ("Keyboard", "input-keyboard-symbolic", "Shortcuts"),
            ],
            search_text: None,
        },
        build,
    );
}
fn group(body: &gtk::Box, name: &str) -> gtk::Box {
    let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 12);
    wrapper.add_css_class("settings-group");
    wrapper.set_widget_name(name);
    wrapper.append(&widgets::label(&name.to_uppercase(), "group-title"));
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("group-list");
    wrapper.append(&rows);
    body.append(&wrapper);
    rows
}
fn build() -> (gtk::Widget, gtk::Box) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let rows = group(&body, "Git window");
    rows.append(&widgets::start_row());
    let p = prefs::get();
    let options = nyx_theme::all().iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let (row, dropdown) = widgets::choice_row("Theme", "Choose the colors used across Nyx apps.", options, &p.theme, |id| {
        prefs::update(|p| {
            p.theme = id;
            p.mode = prefs::ThemeMode::Theme;
        });
        nyx_theme::apply();
    });
    dropdown.set_sensitive(p.mode == prefs::ThemeMode::Theme || !nyx_theme::omarchy_available());
    if nyx_theme::omarchy_available() {
        let (follow, _) = widgets::switch_row(
            "Follow Omarchy theme",
            "Update colors live when the desktop theme changes.",
            p.mode == prefs::ThemeMode::Omarchy,
            move |on| {
                prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                dropdown.set_sensitive(!on);
                nyx_theme::apply();
            },
        );
        rows.append(&follow);
    }
    rows.append(&row);
    let colors = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let refresh = |colors: &gtk::Box| {
        while let Some(child) = colors.first_child() {
            colors.remove(&child);
        }
        let palette = nyx_theme::palette();
        for color in [&palette.bg, &palette.surface, &palette.muted, &palette.text, &palette.accent, &palette.danger] {
            let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            swatch.add_css_class("swatch");
            widgets::paint(&swatch, color);
            swatch.set_tooltip_text(Some(color));
            colors.append(&swatch);
        }
    };
    refresh(&colors);
    let weak = colors.downgrade();
    nyx_theme::subscribe(&colors, move || {
        if let Some(colors) = weak.upgrade() {
            refresh(&colors);
        }
    });
    rows.append(&widgets::row("Current colors", "", Some(colors.upcast_ref())));
    rows.append(&widgets::accent_row("git"));
    let (row, _) = widgets::switch_row("Reduce motion", "Turn off interface animations.", p.reduce_motion, |on| {
        prefs::update(|p| p.reduce_motion = on);
        nyx_theme::apply();
    });
    rows.append(&row);
    let (row, _) = widgets::switch_row(
        "Show scrollbars",
        "Show scrollbars in Git views and dialogs. Scrolling works when hidden.",
        p.show_scrollbars,
        |on| {
            prefs::update(|p| p.show_scrollbars = on);
            super::scrollbars::apply();
        },
    );
    rows.append(&row);
    let rows = group(&body, "Keyboard");
    for (keys, title) in SHORTCUTS {
        rows.append(&widgets::row(title, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .child(&body)
        .build();
    (scroll.upcast(), body)
}
