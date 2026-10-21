use gtk::prelude::*;
use sourceview5::prelude::*;
use std::{cell::RefCell, rc::Rc};

pub struct DiffView {
    pub root: gtk::ScrolledWindow,
    pub buffer: sourceview5::Buffer,
    /// The file's own line numbers ("old new") for each line of the diff, for the gutter.
    numbers: Rc<RefCell<Vec<String>>>,
    gutter: sourceview5::GutterRendererText,
}
impl DiffView {
    pub fn new() -> Self {
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_syntax(false);
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_editable(false);
        view.upcast_ref::<gtk::TextView>().update_property(&[gtk::accessible::Property::Label("File diff")]);
        view.set_cursor_visible(true);
        view.set_monospace(true);
        view.set_left_margin(10);
        view.set_right_margin(10);
        view.add_css_class("git-diff");
        // The patch's own line numbers mean nothing to a reader; show the file's, old and new.
        let numbers = Rc::new(RefCell::new(Vec::<String>::new()));
        let gutter = sourceview5::GutterRendererText::new();
        gutter.set_xalign(1.0);
        gutter.set_xpad(4);
        let lines = numbers.clone();
        gutter.connect_query_data(move |renderer, _, line| {
            if let Some(renderer) = renderer.downcast_ref::<sourceview5::GutterRendererText>() {
                renderer.set_text(lines.borrow().get(line as usize).map_or("", String::as_str));
            }
        });
        sourceview5::prelude::ViewExt::gutter(&view, gtk::TextWindowType::Left).insert(&gutter, 0);
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(super::scrollbars::policy())
            .vscrollbar_policy(super::scrollbars::policy())
            .hexpand(true)
            .vexpand(true)
            .min_content_height(100)
            .child(&view)
            .build();
        for name in ["addition", "deletion", "hunk", "meta"] {
            buffer.create_tag(Some(name), &[]);
        }
        let me = Self { root, buffer, numbers, gutter };
        me.recolor();
        me
    }
    pub fn recolor(&self) {
        let palette = nixe_theme::palette();
        if let Some(scheme) = scheme(&palette) {
            self.buffer.set_style_scheme(Some(&scheme));
        }
        let rgba = |c: &str| gtk::gdk::RGBA::parse(c).unwrap_or(gtk::gdk::RGBA::BLACK);
        let (bg, accent, danger) = (rgba(&palette.bg), rgba(&palette.accent), rgba(&palette.danger));
        let tint = |c: &gtk::gdk::RGBA, t: f32| {
            gtk::gdk::RGBA::new(
                bg.red() * (1.0 - t) + c.red() * t,
                bg.green() * (1.0 - t) + c.green() * t,
                bg.blue() * (1.0 - t) + c.blue() * t,
                1.0,
            )
        };
        let table = self.buffer.tag_table();
        if let Some(tag) = table.lookup("addition") {
            tag.set_foreground_rgba(Some(&accent));
            tag.set_paragraph_background_rgba(Some(&tint(&accent, 0.10)));
        }
        if let Some(tag) = table.lookup("deletion") {
            tag.set_foreground_rgba(Some(&danger));
            tag.set_paragraph_background_rgba(Some(&tint(&danger, 0.10)));
        }
        for name in ["hunk", "meta"] {
            if let Some(tag) = table.lookup(name) {
                tag.set_foreground(Some(&palette.dim_text));
            }
        }
    }
    pub fn set_text(&self, text: &str) {
        let current = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
        // The same diff again (a refresh) keeps the scroll position and the selection.
        if current == text {
            return;
        }
        let adjustment = self.root.vadjustment();
        let scroll = adjustment.value();
        let keep = !current.is_empty() && lines_match(&current, text);
        self.buffer.set_text(text);
        let (kinds, numbers) = annotate(text);
        for (line, kind) in kinds.into_iter().enumerate() {
            let Some(kind) = kind else { continue };
            if let Some(start) = self.buffer.iter_at_line(line as i32) {
                let mut end = start;
                end.forward_to_line_end();
                self.buffer.apply_tag_by_name(kind, &start, &end);
            }
        }
        let width = numbers.iter().map(String::len).max().unwrap_or(0);
        *self.numbers.borrow_mut() = numbers;
        let (w, _) = sourceview5::prelude::GutterRendererTextExt::measure(&self.gutter, &"0".repeat(width.max(1)));
        self.gutter.set_size_request(if width == 0 { 0 } else { w + 8 }, -1);
        self.gutter.set_visible(width > 0);
        self.gutter.queue_draw();
        // A changed diff of the same file stays where the reader was.
        if keep {
            gtk::glib::idle_add_local_once(move || adjustment.set_value(scroll));
        }
    }
    pub fn range(&self) -> Option<(usize, usize)> {
        self.buffer
            .selection_bounds()
            .map(|(start, end)| (start.line() as usize, end.line() as usize + usize::from(end.line_offset() > 0)))
    }
    pub fn cursor_line(&self) -> usize {
        self.buffer.iter_at_offset(self.buffer.cursor_position()).line() as usize
    }
}

/// Two diffs of the same file: their first lines (`diff --git a/… b/…`) agree.
fn lines_match(a: &str, b: &str) -> bool {
    a.lines().next() == b.lines().next()
}

/// Each line's tag, and its old and new line numbers in the file, as narrow as they fit.
fn annotate(text: &str) -> (Vec<Option<&'static str>>, Vec<String>) {
    let mut kinds = Vec::new();
    let mut lines: Vec<Option<(Option<u64>, Option<u64>)>> = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    let mut in_hunk = false;
    let mut header = false;
    let mut any = false;
    for line in text.lines() {
        let (kind, number) = if line.starts_with("diff ") {
            in_hunk = false;
            header = true;
            (Some("meta"), None)
        } else if line.starts_with("@@") {
            // @@ -old,count +new,count @@
            let mut parts = line.split_whitespace().skip(1);
            let start = |p: Option<&str>| p.and_then(|p| p.get(1..)?.split(',').next()?.parse().ok()).unwrap_or(0);
            old = start(parts.next());
            new = start(parts.next());
            in_hunk = true;
            any = true;
            (Some("hunk"), None)
        } else if !in_hunk {
            header |= line.starts_with("--- ");
            (header.then_some("meta"), None)
        } else if line.starts_with('+') {
            new += 1;
            (Some("addition"), Some((None, Some(new - 1))))
        } else if line.starts_with('-') {
            old += 1;
            (Some("deletion"), Some((Some(old - 1), None)))
        } else if line.starts_with('\\') {
            (Some("meta"), None)
        } else {
            old += 1;
            new += 1;
            (None, Some((Some(old - 1), Some(new - 1))))
        };
        kinds.push(kind);
        lines.push(number);
    }
    if !any {
        return (kinds, Vec::new());
    }
    let digits = (old.max(new).max(1) as f64).log10().floor() as usize + 1;
    let column = |n: Option<u64>| n.map_or_else(|| " ".repeat(digits), |n| format!("{n:>digits$}"));
    let numbers =
        lines.into_iter().map(|n| n.map_or_else(String::new, |(o, n)| format!("{} {}", column(o), column(n)))).collect();
    (kinds, numbers)
}

// GtkSourceView paints its gutter from a style scheme, independently of GTK CSS.
// Cache a scheme generated from the same semantic palette as the window.
fn scheme(palette: &nixe_theme::Palette) -> Option<sourceview5::StyleScheme> {
    use std::cell::RefCell;
    thread_local! {static LAST: RefCell<Option<(nixe_theme::Palette, sourceview5::StyleScheme)>> = const {RefCell::new(None)};}
    LAST.with(|last| {
        if let Some((old, scheme)) = last.borrow().as_ref()
            && old == palette
        {
            return Some(scheme.clone());
        }
        let directory = nixe_core::paths::cache_dir().join("sourceview");
        std::fs::create_dir_all(&directory).ok()?;
        let bg = gtk::glib::markup_escape_text(&palette.bg);
        let text = gtk::glib::markup_escape_text(&palette.text);
        let dim = gtk::glib::markup_escape_text(&palette.dim_text);
        let accent = gtk::glib::markup_escape_text(&palette.accent);
        let bg_rgba = gtk::gdk::RGBA::parse(&palette.bg).ok()?;
        let accent_rgba = gtk::gdk::RGBA::parse(&palette.accent).ok()?;
        let channel = |a: f32, b: f32| ((a * 0.82 + b * 0.18) * 255.0).round() as u8;
        let selection = format!(
            "#{:02x}{:02x}{:02x}",
            channel(bg_rgba.red(), accent_rgba.red()),
            channel(bg_rgba.green(), accent_rgba.green()),
            channel(bg_rgba.blue(), accent_rgba.blue())
        );
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="nixe-git-diff" name="Git" version="1.0">
<style name="text" foreground="{text}" background="{bg}"/>
<style name="line-numbers" foreground="{dim}" background="{bg}"/>
<style name="current-line-number" foreground="{accent}" background="{bg}"/>
<style name="current-line" background="{bg}"/>
<style name="selection" foreground="{text}" background="{selection}"/>
<style name="cursor" foreground="{accent}"/>
</style-scheme>"#
        );
        nixe_core::cmd::atomic_write(&directory.join("nixe-git-diff.xml"), &xml).ok()?;
        let manager = sourceview5::StyleSchemeManager::new();
        manager.append_search_path(&directory.to_string_lossy());
        manager.force_rescan();
        let scheme = manager.scheme("nixe-git-diff")?;
        *last.borrow_mut() = Some((palette.clone(), scheme.clone()));
        Some(scheme)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_lines_from_hunk_headers() {
        let (kinds, numbers) = annotate("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -10,3 +10,3 @@\n a\n-b\n+c\n");
        assert_eq!(kinds, [Some("meta"), Some("meta"), Some("meta"), Some("hunk"), None, Some("deletion"), Some("addition")]);
        assert_eq!(numbers[4].split_whitespace().collect::<Vec<_>>(), ["10", "10"]);
        assert_eq!(numbers[5].trim(), "11");
        assert_eq!(numbers[6].trim(), "11");
        assert!(annotate("Select a file.").1.is_empty());
    }
}
