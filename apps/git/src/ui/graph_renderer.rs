use gtk::{cairo, gdk, pango, prelude::*};
use nyx_git::graph::model::{Query, RefKind, Row};
pub fn row_height(zoom: f64) -> f64 {
    36.0 * zoom
}
pub fn relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
    let delta = now.saturating_sub(timestamp);
    if delta < 0 {
        "future".into()
    } else if delta < 60 {
        "now".into()
    } else if delta < 3600 {
        format!("{}m", delta / 60)
    } else if delta < 86400 {
        format!("{}h", delta / 3600)
    } else if delta < 86400 * 30 {
        format!("{}d", delta / 86400)
    } else if delta < 86400 * 365 {
        format!("{}mo", delta / (86400 * 30))
    } else {
        format!("{}y", delta / (86400 * 365))
    }
}
fn color(text: &str) -> gdk::RGBA {
    gdk::RGBA::parse(text).unwrap_or(gdk::RGBA::BLACK)
}
fn mix(a: &gdk::RGBA, b: &gdk::RGBA, t: f32) -> gdk::RGBA {
    gdk::RGBA::new(
        a.red() * (1.0 - t) + b.red() * t,
        a.green() * (1.0 - t) + b.green() * t,
        a.blue() * (1.0 - t) + b.blue() * t,
        1.0,
    )
}
fn source(cr: &cairo::Context, color: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, alpha);
}
fn font(area: &gtk::DrawingArea, zoom: f64) -> pango::FontDescription {
    let mut font =
        area.pango_context().font_description().unwrap_or_else(|| pango::FontDescription::from_string("JetBrains Mono"));
    font.set_absolute_size(12.0 * zoom * pango::SCALE as f64);
    font
}
/// Draws lines of text in the graph's font, each ellipsized to a width and centred on a y.
struct Text<'a> {
    area: &'a gtk::DrawingArea,
    cr: &'a cairo::Context,
    font: &'a pango::FontDescription,
}
impl Text<'_> {
    fn draw(&self, value: &str, x: f64, y: f64, width: f64, color: &gdk::RGBA, alpha: f64) {
        if width < 8.0 {
            return;
        }
        let layout = self.area.create_pango_layout(Some(value));
        layout.set_font_description(Some(self.font));
        layout.set_width((width * pango::SCALE as f64) as i32);
        layout.set_single_paragraph_mode(true);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        source(self.cr, color, alpha);
        self.cr.move_to(x, y - layout.pixel_size().1 as f64 / 2.0);
        pangocairo::functions::show_layout(self.cr, &layout);
    }
}
pub struct Frame<'a> {
    pub rows: &'a [Row],
    pub start: usize,
    pub offset: f64,
    pub zoom: f64,
    pub selected: Option<usize>,
    pub hover: Option<usize>,
    pub lanes: usize,
    pub query: &'a Query,
    /// The commit marked for comparison.
    pub comparison: Option<&'a str>,
}
pub fn draw(area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64, frame: Frame<'_>) {
    let palette = nyx_theme::palette();
    let bg = color(&palette.bg);
    let accent = color(&palette.accent);
    let danger = color(&palette.danger);
    let fg = color(&palette.text);
    let dim = color(&palette.dim_text);
    let colors = [
        accent,
        mix(&accent, &danger, 0.6),
        mix(&accent, &fg, 0.6),
        danger,
        mix(&accent, &dim, 0.55),
        mix(&danger, &fg, 0.5),
        mix(&accent, &danger, 0.25),
        mix(&dim, &fg, 0.6),
    ];
    source(cr, &bg, 1.0);
    let _ = cr.paint();
    let row_h = row_height(frame.zoom);
    let font = font(area, frame.zoom);
    let write = Text { area, cr, font: &font };
    let graph_w = (frame.lanes as f64 * 18.0 * frame.zoom + 24.0).min(width * 0.4).max(48.0);
    let spacing = (18.0 * frame.zoom).min((graph_w - 24.0) / frame.lanes.max(1) as f64);
    let x = |lane: usize| 16.0 + lane as f64 * spacing;
    cr.rectangle(0.0, 0.0, width, height);
    cr.clip();
    cr.set_line_width(2.0 * frame.zoom);
    cr.set_line_cap(cairo::LineCap::Round);
    for (i, row) in frame.rows.iter().enumerate() {
        let index = frame.start + i;
        let top = index as f64 * row_h - frame.offset;
        let middle = top + row_h / 2.0;
        if top + row_h < 0.0 || top > height {
            continue;
        }
        if frame.selected == Some(index) || frame.hover == Some(index) {
            source(cr, &mix(&bg, &accent, if frame.selected == Some(index) { 0.18 } else { 0.07 }), 1.0);
            cr.rectangle(0.0, top, width, row_h);
            let _ = cr.fill();
        }
        if frame.comparison == Some(row.commit.id.as_str()) {
            source(cr, &accent, 1.0);
            cr.rectangle(0.0, top + 3.0, 3.0, row_h - 6.0);
            let _ = cr.fill();
        }
        for edge in &row.edges {
            let from = top + row_h * edge.top as f64 / 2.0;
            let to = top + row_h * edge.bottom as f64 / 2.0;
            source(cr, &colors[edge.color % colors.len()], 0.9);
            cr.move_to(x(edge.from), from);
            cr.curve_to(x(edge.from), (from + to) / 2.0, x(edge.to), (from + to) / 2.0, x(edge.to), to);
            let _ = cr.stroke();
        }
        source(cr, &bg, 1.0);
        cr.arc(x(row.lane), middle, 6.0 * frame.zoom, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let is_head = row.commit.refs.iter().any(|r| r.kind == RefKind::Head);
        source(cr, &colors[row.color % colors.len()], 1.0);
        cr.arc(x(row.lane), middle, if is_head { 5.5 * frame.zoom } else { 4.2 * frame.zoom }, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        if row.commit.parents.len() > 1 || is_head || row.commit.shallow {
            source(cr, &bg, 1.0);
            cr.arc(x(row.lane), middle, 2.3 * frame.zoom, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        let match_alpha = if frame.query.is_empty() || frame.query.matches(&row.commit) { 1.0 } else { 0.38 };
        let mut left = graph_w + 10.0;
        let author_width = if width > 720.0 { 120.0 * frame.zoom } else { 0.0 };
        let time_width = 45.0 * frame.zoom;
        let right = (width - author_width - time_width - 18.0).max(left + 8.0);
        if !row.commit.refs.is_empty() {
            let names = row
                .commit
                .refs
                .iter()
                .map(|r| match r.kind {
                    RefKind::Tag => format!("tag:{}", r.name),
                    RefKind::Remote => format!("↗{}", r.name),
                    _ => r.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let label = area.create_pango_layout(Some(&names));
            label.set_font_description(Some(&font));
            let chip_width = (label.pixel_size().0 as f64 + 14.0).min((right - left) * 0.5).max(0.0);
            source(cr, &mix(&bg, &accent, 0.18), 1.0);
            cr.rectangle(left, middle - 11.0 * frame.zoom, chip_width, 22.0 * frame.zoom);
            let _ = cr.fill();
            write.draw(&names, left + 5.0, middle, chip_width - 10.0, &accent, match_alpha);
            left += chip_width + 8.0;
        }
        write.draw(&row.commit.subject, left, middle, right - left, &fg, match_alpha);
        if author_width > 0.0 {
            write.draw(
                &row.commit.author,
                width - author_width - time_width - 8.0,
                middle,
                author_width - 12.0,
                &dim,
                match_alpha,
            );
        }
        write.draw(&relative_time(row.commit.timestamp), width - time_width - 6.0, middle, time_width, &dim, match_alpha);
    }
}
