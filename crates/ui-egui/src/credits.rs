//! The About window's credits: every contributor (GitHub login always; display and real names only
//! with recorded consent) and the AI models that helped. The tables are baked into the binary by
//! build.rs through craft-credits from contributors/contributors.json (craftrules
//! standards/contributors.md); nothing is read at run time. The data, sorting and formatting come
//! from craft-credits; UI strings go through `tl!` (column abbreviations like `+LOC` stay as they are).

use craft_credits::fmt::group;
use craft_credits::{Contributor, Model, NameMode, SortKey, sorted};
use egui::RichText;

use crate::PhotocraftApp;

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// A table header in the UI language. The symbols (`PRs`, `+LOC`, …) stay as they are.
fn header_label(k: SortKey) -> &'static str {
    let h = k.label().1;
    if craft_credits::UI_STRINGS.contains(&h) { tl!(h) } else { h }
}

/// One line with everything we know about `c`, for tooltips.
fn summary(c: &Contributor) -> String {
    c.summary_in(tl!(craft_credits::SUMMARY))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    names: NameMode,
    key: SortKey,
    ascending: bool,
    table: bool,
}

impl Default for View {
    fn default() -> Self {
        View { names: NameMode::Username, key: SortKey::FirstCommit, ascending: true, table: false }
    }
}

/// Height of the Contributors list (grab bag or table, header included), the same on both views so
/// switching between them doesn't resize the window. The Models table is as tall as its rows, up
/// to this.
const LIST_HEIGHT: f32 = 340.0;
const TABLE_HEADER_H: f32 = 24.0;
const TABLE_ROW_H: f32 = 22.0;
/// Room for the sort arrow in every header, so the columns don't move when the sort does.
const SORT_ARROW_W: f32 = 12.0;
/// Padding inside each cell, on each side.
const CELL_PAD: f32 = 6.0;
const NAME_MIN_W: f32 = 110.0;

/// About ▸ Contributors: a name toggle, a sort, and the list as a grab bag or a table.
pub fn contributors_ui(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    contributors_in(app, ui, CONTRIBUTORS, TOTAL_COMMITS);
}

/// [`contributors_ui`] over a given list (tests pass their own).
fn contributors_in(app: &mut PhotocraftApp, ui: &mut egui::Ui, contributors: &[Contributor], total_commits: u64) {
    let id = egui::Id::new("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(tl!("Show"));
        for m in NameMode::ALL {
            if ui.selectable_label(v.names == m, tl!(m.label())).clicked() {
                v.names = m;
            }
        }
        ui.separator();
        ui.label(tl!("Sort"));
        let (press, close) = crate::press_menu::PressCombo::before(ui, "credits_sort");
        let sort = egui::ComboBox::from_id_salt("credits_sort").selected_text(tl!(v.key.label().0)).close_behavior(close).show_ui(ui, |ui| {
            for k in SortKey::ALL {
                let item = ui.selectable_label(v.key == k, tl!(k.label().0));
                if press.chosen(ui, &item) {
                    v.key = k;
                    v.ascending = k.default_ascending();
                }
            }
        });
        press.after(&sort.response);
        if ui.button(if v.ascending { "▲" } else { "▼" }).on_hover_text(tl!("Reverse the order")).clicked() {
            v.ascending = !v.ascending;
        }
        ui.separator();
        if ui.selectable_label(!v.table, tl!("Grab bag")).clicked() {
            v.table = false;
        }
        if ui.selectable_label(v.table, tl!("Table")).clicked() {
            v.table = true;
        }
    });
    let list = sorted(contributors, v.names, v.key, v.ascending);
    let counts = crate::i18n::fmt(
        tl!("Contributors: {contributors} · Commits: {commits}"),
        &[("contributors", &group(list.len() as u64)), ("commits", &group(total_commits))],
    );
    ui.label(RichText::new(counts).small().weak());
    ui.separator();
    let height = LIST_HEIGHT;
    if list.is_empty() {
        ui.label(tl!("No contributor data was built into this copy."));
    } else if v.table {
        table(app, ui, &list, &mut v, height);
    } else {
        egui::ScrollArea::vertical().id_salt("credits_bag").auto_shrink([false, false]).max_height(height).show(ui, |ui| {
            // Spacing alone separates the names: a "·" between them started some lines.
            ui.spacing_mut().item_spacing = egui::vec2(14.0, 8.0);
            // Each name is one unit: otherwise egui flows a link that doesn't fit onto the next
            // line as wrapped text, at the font's line height instead of the row spacing.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // horizontal_wrapped gives its first row the interact height; text rows are shorter,
            // so the first gap came out wider than the rest.
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal_wrapped(|ui| {
                for c in &list {
                    link(app, ui, c, v.names);
                }
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(id, v));
}

/// Height of a table showing all `rows` (header included), up to [`LIST_HEIGHT`].
fn table_height(ui: &egui::Ui, rows: usize) -> f32 {
    // Each row, and the header, is followed by the row gap.
    let gap = ui.spacing().item_spacing.y;
    let body = rows as f32 * (TABLE_ROW_H + gap) + gap;
    (TABLE_HEADER_H + body).min(LIST_HEIGHT)
}

/// A contributor's name, linking to their GitHub profile. Routes through [`crate::links::open`]
/// (the platform browser service) rather than `ui.hyperlink_to`'s `ctx.open_url`, which the
/// desktop build can't rely on (egui_winit needs its "links" feature, which we don't enable).
fn link(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &Contributor, names: NameMode) -> egui::Response {
    let url = format!("https://github.com/{}", c.login);
    let r = ui.add(egui::Link::new(c.name(names))).on_hover_text(summary(c));
    if r.clicked() {
        crate::links::open(app, ui.ctx(), &url);
    }
    r
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE).size().x)
}

/// A column's width: its widest cell, or its header plus the sort arrow's room, so no column
/// changes width when the sort or the rows change.
fn column_width<'a>(ui: &egui::Ui, header: &str, cells: impl Iterator<Item = &'a str>) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let widest = cells.map(|c| text_width(ui, c, &body)).fold(0.0, f32::max);
    let head = text_width(ui, header, &crate::theme::semibold(body.size)) + SORT_ARROW_W;
    (widest.max(head) + 2.0 * CELL_PAD).ceil()
}

/// A credits table: columns of `widths`, the `None` one taking what the others leave (at least
/// `flex_min`). The header stays put while the rows scroll, within `height` (header included). Too
/// narrow for every column (a long translation), it scrolls sideways rather than clip one.
fn credits_table(
    ui: &mut egui::Ui,
    id: &str,
    widths: &[Option<f32>],
    flex_min: f32,
    height: f32,
    header: impl FnOnce(egui_extras::TableRow<'_, '_>),
    body: impl FnOnce(egui_extras::TableBody<'_>),
) {
    // The rows' scroll bar sits beside the columns: leave it room, or it pushes the table past the
    // window and a sideways scroll bar appears under it.
    let fixed: f32 = widths.iter().flatten().sum::<f32>() + ui.spacing().scroll.allocated_width();
    // The table puts a row gap under its header.
    let body_h = (height - TABLE_HEADER_H - ui.spacing().item_spacing.y).max(TABLE_ROW_H);
    egui::ScrollArea::horizontal().id_salt((id, "h")).auto_shrink([false, true]).show(ui, |ui| {
        let flex = (ui.available_width() - fixed).max(flex_min);
        // No gaps between cells: their padding spaces them, and the header rule runs unbroken.
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut b = egui_extras::TableBuilder::new(ui)
            .id_salt(id)
            .striped(true)
            .resizable(false)
            .auto_shrink([false, false])
            .min_scrolled_height(body_h)
            .max_scroll_height(body_h)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        for w in widths {
            b = b.column(match w {
                Some(w) => egui_extras::Column::exact(*w),
                None => egui_extras::Column::exact(flex).clip(true),
            });
        }
        b.header(TABLE_HEADER_H, header).body(body);
    });
}

/// The Contributors table; clicking a header sorts by it (again reverses it).
fn table(app: &mut PhotocraftApp, ui: &mut egui::Ui, list: &[&Contributor], v: &mut View, height: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let names = v.names;
    let widths: Vec<Option<f32>> = SortKey::ALL
        .iter()
        .map(|&k| {
            let cells: Vec<String> = list.iter().map(|c| k.cell(c)).collect();
            (k != SortKey::Name).then(|| column_width(ui, header_label(k), cells.iter().map(String::as_str)))
        })
        .collect();
    credits_table(
        ui,
        "credits_table",
        &widths,
        NAME_MIN_W,
        height,
        |mut header| {
            for k in SortKey::ALL {
                header.col(|ui| {
                    let sort = (v.key == k).then_some(v.ascending);
                    let (long, short) = (tl!(k.label().0), header_label(k));
                    if header_cell(ui, &t, short, k.numeric(), Some(sort)).on_hover_text(long).clicked() {
                        if v.key == k {
                            v.ascending = !v.ascending;
                        } else {
                            v.key = k;
                            v.ascending = k.default_ascending();
                        }
                    }
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, list.len(), |mut row| {
                let Some(c) = list.get(row.index()) else { return };
                row.col(|ui| {
                    ui.add_space(CELL_PAD);
                    link(app, ui, c, names);
                });
                for k in SortKey::ALL.into_iter().skip(1) {
                    row.col(|ui| cell(ui, k.numeric(), &k.cell(c)));
                }
            });
        },
    );
}

fn cell(ui: &mut egui::Ui, numeric: bool, text: &str) {
    let layout = if numeric { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
    ui.with_layout(layout, |ui| {
        ui.add_space(CELL_PAD);
        ui.add(egui::Label::new(text).selectable(false));
    });
}

/// One header: its label (on the right over counts, like their numbers) under a rule. A sortable
/// one (`sort` is `Some`) is a button, with the arrow on the label's inner side when it's the sort
/// (`Some(Some(ascending))`), in room every header keeps.
fn header_cell(ui: &mut egui::Ui, t: &crate::theme::Tokens, label: &str, numeric: bool, sort: Option<Option<bool>>) -> egui::Response {
    let rect = ui.max_rect();
    let resp = ui.allocate_rect(rect, if sort.is_some() { egui::Sense::click() } else { egui::Sense::hover() });
    let kind = if sort.is_some() { egui::WidgetType::Button } else { egui::WidgetType::Label };
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, true, label));
    let resp = if sort.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let current = sort.flatten();
    let color = if sort.is_none() || current.is_some() || resp.hovered() { t.text } else { t.text_dim };
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    let galley = ui.painter().layout_no_wrap(label.to_string(), crate::theme::semibold(size), color);
    let inner = rect.shrink2(egui::vec2(CELL_PAD, 0.0));
    let y = rect.center().y - galley.size().y / 2.0;
    let (text_x, arrow_x) = if numeric {
        let x = inner.right() - galley.size().x;
        (x, x - SORT_ARROW_W / 2.0)
    } else {
        (inner.left(), inner.left() + galley.size().x + SORT_ARROW_W / 2.0)
    };
    ui.painter().galley(egui::pos2(text_x, y), galley, color);
    if let Some(ascending) = current {
        let (c, h) = (egui::pos2(arrow_x, rect.center().y), 3.0);
        let pts = if ascending {
            vec![c + egui::vec2(-h, h * 0.6), c + egui::vec2(h, h * 0.6), c + egui::vec2(0.0, -h * 0.9)]
        } else {
            vec![c + egui::vec2(-h, -h * 0.6), c + egui::vec2(h, -h * 0.6), c + egui::vec2(0.0, h * 0.9)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, t.accent, egui::Stroke::NONE));
    }
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 1.0), rect.max);
    ui.painter().rect_filled(line, 0.0, t.separator);
    resp
}

/// About ▸ Models: AI models credited in Co-Authored-By trailers.
pub fn models_ui(ui: &mut egui::Ui) {
    models_in(ui, MODELS, TOTAL_COMMITS);
}

/// Share of all commits, whole percent; a model with any commits never reads "0%".
fn percent(commits: u64, total: u64) -> String {
    let p = 100.0 * commits as f64 / total.max(1) as f64;
    if commits > 0 && p < 0.5 { "<1%".into() } else { format!("{:.0}%", p.min(100.0)) }
}

/// The Models table's cells for `m`, column by column; counts are the last three.
fn model_cells(m: &Model, total: u64) -> [String; 6] {
    [
        m.company.to_string(),
        m.model.to_string(),
        m.version.to_string(),
        group(m.commits),
        percent(m.commits, total),
        format!("+{} / −{}", group(m.lines_added), group(m.lines_deleted)),
    ]
}

/// [`models_ui`] over a given list (tests pass their own).
fn models_in(ui: &mut egui::Ui, models: &[Model], total_commits: u64) {
    if models.is_empty() {
        ui.label(tl!("No model credits were built into this copy."));
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    // Counted against every commit, or the busiest model's if the total somehow trails it.
    let total = total_commits.max(models.iter().map(|m| m.commits).max().unwrap_or(0));
    let heads = [tl!("Company"), tl!("Model"), tl!("Version"), tl!("Commits"), tl!("% of all commits"), tl!("Lines +/−")];
    let rows: Vec<[String; 6]> = models.iter().map(|m| model_cells(m, total)).collect();
    // The model name takes the spare width.
    let widths: Vec<Option<f32>> =
        (0..heads.len()).map(|i| (i != 1).then(|| column_width(ui, heads[i], rows.iter().filter_map(|r| r.get(i)).map(String::as_str)))).collect();
    let numeric = |i: usize| i >= 3;
    credits_table(
        ui,
        "credits_models",
        &widths,
        NAME_MIN_W,
        table_height(ui, rows.len()),
        |mut header| {
            for (i, h) in heads.iter().enumerate() {
                header.col(|ui| {
                    header_cell(ui, &t, h, numeric(i), None);
                });
            }
        },
        |body| {
            body.rows(TABLE_ROW_H, rows.len(), |mut row| {
                let Some(cells) = rows.get(row.index()) else { return };
                for (i, text) in cells.iter().enumerate() {
                    row.col(|ui| cell(ui, numeric(i), text));
                }
            });
        },
    );
}

#[cfg(test)]
#[path = "credits_tests.rs"]
mod ui_tests;
