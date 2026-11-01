use super::buttons::DROPDOWN;
use super::tooltip::{keytip_badge, show_hint};
use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, Tokens, Ui, icons, pos2, regular, vec2};

/// Word's standard colour grid (theme colours + tints, standard colours). Returns a picked hex.
pub fn color_grid(ui: &mut Ui, theme: &[wordcraft_doc::Rgb]) -> Option<String> {
    let mut picked = None;
    ui.label(egui::RichText::new(tl!("Theme Colors")).small().weak());
    let base: Vec<wordcraft_doc::Rgb> = theme.iter().take(10).copied().collect();
    let tint = |c: wordcraft_doc::Rgb, k: f32| {
        let f = |v: u8| if k >= 0.0 { (v as f32 + (255.0 - v as f32) * k) as u8 } else { (v as f32 * (1.0 + k)) as u8 };
        wordcraft_doc::Rgb(f(c.0), f(c.1), f(c.2))
    };
    let order = [1usize, 0, 3, 2, 4, 5, 6, 7, 8, 9];
    for k in [0.0f32, 0.8, 0.6, 0.4, -0.25, -0.5] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, if k == 0.0 { 6.0 } else { 0.0 });
            for i in order {
                let Some(c) = base.get(i).copied() else { continue };
                let c = tint(c, k);
                if swatch(ui, c) {
                    picked = Some(c.hex());
                }
            }
        });
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Standard Colors")).small().weak());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
        for hex in ["C00000", "FF0000", "FFC000", "FFFF00", "92D050", "00B050", "00B0F0", "0070C0", "002060", "7030A0"] {
            if let Some(c) = wordcraft_doc::Rgb::parse(hex)
                && swatch(ui, c)
            {
                picked = Some(hex.to_string());
            }
        }
    });
    picked
}

pub(crate) fn swatch(ui: &mut Ui, c: wordcraft_doc::Rgb) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::click());
    ui.painter().rect_filled(r, 0.0, Color32::from_rgb(c.0, c.1, c.2));
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(
        r,
        0.0,
        Stroke::new(if resp.hovered() { 1.5 } else { 0.5 }, if resp.hovered() { t.accent } else { t.border_strong }),
        egui::StrokeKind::Inside,
    );
    resp.on_hover_text(format!("#{}", c.hex())).clicked()
}

/// A ribbon-style text field with a dropdown list. Returns a chosen/typed value; Enter commits
/// the typed text as is (the font size box relies on that: "1" must not become "10").
pub fn combo(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, ComboOptions { id, width, current, items, recent: &[], preview, search: false, tip: "" })
}

/// [`combo`] whose field and arrow show the hover hint `tip` (already translated).
pub fn combo_hint(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
    tip: &str,
) -> Option<String> {
    combo_impl(ui, ComboOptions { id, width, current, items, recent: &[], preview, search: false, tip })
}

/// What [`font_combo`] returns when its list's last entry, Add Font…, is chosen (#582): the
/// caller runs `font.add`, which shows a file picker. Not a family name anyone can type.
pub const ADD_FONT: &str = "\u{1}font.add";

/// What [`font_combo`] returns when "Load System Installed Fonts…" is chosen on web: the
/// caller runs `font.loadSystem`.
pub const LOAD_SYSTEM_FONTS: &str = "\u{1}font.loadSystem";

/// The font box: a [`combo`] where typing lists the matching families under the field (#359).
/// Its list ends with Add Font… ([`ADD_FONT`]).
/// Up/Down move the highlight, Enter or a click applies the highlighted family ("arial" applies
/// "Arial"), Escape restores the current font. Text matching no family is applied as typed, as
/// Word allows fonts that are not installed.
pub fn font_combo(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, ComboOptions { id, width, current, items, recent: &[], preview, search: true, tip: tl!("Font") })
}

/// [`font_combo`] with the `recent` families (most recent first) in a Recently Used Fonts
/// section above the list, then All Fonts, as in Word (#2068). The sections show when nothing is
/// typed; a query lists its matches only. Up/Down walk the recent entries, then the list; the
/// headings are skipped. A recent font that is no longer installed stays listed and is applied as
/// typed.
pub fn font_combo_with_recent(
    ui: &mut Ui,
    id: &str,
    width: f32,
    current: &str,
    items: &[String],
    recent: &[String],
    preview: Option<&dyn Fn(&mut Ui, &str) -> Response>,
) -> Option<String> {
    combo_impl(ui, ComboOptions { id, width, current, items, recent, preview, search: true, tip: tl!("Font") })
}

/// [`font_combo_with_recent`] with a hover hint `tip` (already translated).
pub struct FontComboOptions<'a> {
    pub id: &'a str,
    pub width: f32,
    pub current: &'a str,
    pub items: &'a [String],
    pub recent: &'a [String],
    pub preview: Option<&'a dyn Fn(&mut Ui, &str) -> Response>,
    pub tip: &'a str,
}

pub fn font_combo_with_recent_hint(ui: &mut Ui, options: FontComboOptions<'_>) -> Option<String> {
    combo_impl(
        ui,
        ComboOptions {
            search: true,
            id: options.id,
            width: options.width,
            current: options.current,
            items: options.items,
            recent: options.recent,
            preview: options.preview,
            tip: options.tip,
        },
    )
}

struct ComboOptions<'a> {
    id: &'a str,
    width: f32,
    current: &'a str,
    items: &'a [String],
    recent: &'a [String],
    preview: Option<&'a dyn Fn(&mut Ui, &str) -> Response>,
    search: bool,
    tip: &'a str,
}

/// Indices of the `items` matching `query`, ignoring case: exact matches, then the names starting
/// with it, then those containing it, each group in list order. A blank query matches everything.
pub fn font_matches(query: &str, items: &[String]) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let (mut exact, mut prefix, mut inner) = (Vec::new(), Vec::new(), Vec::new());
    for (i, it) in items.iter().enumerate() {
        let name = it.to_lowercase();
        if name == q {
            exact.push(i);
        } else if name.starts_with(&q) {
            prefix.push(i);
        } else if name.contains(&q) {
            inner.push(i);
        }
    }
    exact.extend(prefix);
    exact.extend(inner);
    exact
}

/// Width of a combo arrow's border, the field's own (the theme's input border is 1 pt).
const ARROW_BORDER: f32 = 1.0;

/// A searching combo's state between frames.
#[derive(Clone, Default)]
struct ComboSearch {
    /// The match list is showing (the user typed since focusing the field).
    open: bool,
    /// The query `matches` were computed for, and the list length then.
    query: String,
    items_len: usize,
    matches: std::sync::Arc<Vec<usize>>,
    /// Index among the entries: the recent families shown, then `matches`.
    highlight: usize,
    /// Up or Down moved the highlight since the query last changed.
    moved: bool,
    /// Rows the list drew last frame; 0 while it is hidden.
    rows: usize,
    /// Where the list was drawn last frame, its scroll offset and visible height.
    popup: Option<Rect>,
    offset: f32,
    view_h: f32,
}

/// Draws an editable or searchable combo box: keeps temporary text and search state, supports
/// keyboard selection, popup rows with recent and preview sections, an arrow and hint, and
/// returns the chosen or committed text.
fn combo_impl(ui: &mut Ui, options: ComboOptions<'_>) -> Option<String> {
    let ComboOptions { id, width, current, items, recent, preview, search, tip } = options;
    let mut out = None;
    let edit_id = ui.scope_id().with(("combo_text", id));
    let te_id = ui.scope_id().with(("combo_edit", id));
    let search_id = ui.scope_id().with(("combo_search", id));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(edit_id)).unwrap_or_else(|| current.to_string());
    let mut s: ComboSearch = if search { ui.data_mut(|d| d.get_temp(search_id)).unwrap_or_default() } else { ComboSearch::default() };
    let mut focused = false;
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        // The field and its arrow share one outline: rounded on the outside, square where they
        // meet. The field's frame takes the widget visuals' radius in every state.
        let r = ui.visuals().widgets.inactive.corner_radius;
        let w = &mut ui.visuals_mut().widgets;
        for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            v.corner_radius.ne = 0;
            v.corner_radius.se = 0;
        }
        // Up/Down move the list's highlight: take them before the text field moves its cursor.
        let mut step = 0i32;
        if s.open && ui.memory(|m| m.has_focus(te_id)) {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    step += 1;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    step -= 1;
                }
            });
        }
        let te = egui::TextEdit::singleline(&mut text).id(te_id).desired_width(width - 16.0).font(regular(12.0)).margin(vec2(4.0, 3.0));
        // The arrow's box is painted here, under the field, once its size is known.
        let arrow_bg = ui.painter().add(egui::Shape::Noop);
        let resp = ui.add(te);
        // The field says what it is: without a name a screen reader has only "edit" to say
        // (#3172). The arrow says whose options it opens, as the split buttons' arrow does.
        if !tip.is_empty() {
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), tip));
        }
        keytip_badge(ui, resp.rect, "combo", id);
        focused = resp.has_focus();
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut reveal = step != 0;
        if search && resp.changed() {
            s.open = true;
            s.highlight = 0;
            s.moved = false;
            reveal = true;
        }
        // The recent section shows while nothing is typed; a query lists its matches only.
        let recent_shown: &[String] = if text.trim().is_empty() { recent } else { &[] };
        let mut regain_focus = false;
        if s.open {
            if s.query != text || s.items_len != items.len() {
                s.matches = std::sync::Arc::new(font_matches(&text, items));
                s.query = text.clone();
                s.items_len = items.len();
            }
            let last = recent_shown.len().saturating_add(s.matches.len()).saturating_sub(1);
            s.moved |= step != 0;
            s.highlight = match step {
                1.. => s.highlight.saturating_add(1),
                0 => s.highlight,
                _ => s.highlight.saturating_sub(1),
            }
            .min(last);
            if enter {
                let picked = recent_shown
                    .get(s.highlight)
                    .or_else(|| s.highlight.checked_sub(recent_shown.len()).and_then(|k| s.matches.get(k)).and_then(|&i| items.get(i)));
                // A cleared box commits as typed (nothing) unless Up or Down picked an entry.
                out = Some(match picked {
                    Some(name) if !text.trim().is_empty() || s.moved => name.clone(),
                    _ => text.clone(),
                });
                s.open = false;
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Tab)) {
                s.open = false;
            } else if !focused {
                // A click in the list takes focus from the field: keep the list, give focus back.
                let over = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| s.popup.is_some_and(|r| r.contains(p)));
                if over {
                    regain_focus = true;
                } else {
                    s.open = false;
                }
            }
        } else if enter {
            out = Some(text.clone());
        }
        let (ar, aresp) = ui.allocate_exact_size(vec2(16.0, resp.rect.height()), Sense::click());
        if !tip.is_empty() {
            let arrow_name = crate::i18n::fmt(tl!("{name} options"), &[("name", tip)]);
            aresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &arrow_name));
        }
        // It reaches one border width under the field, so the field's right border is the only line
        // between them: two borders side by side read as a double-width divider.
        let ar_radius = CornerRadius { nw: 0, sw: 0, ne: r.ne, se: r.se };
        let under = Rect::from_min_max(pos2(ar.min.x - ARROW_BORDER, ar.min.y), ar.max);
        let fill = if aresp.hovered() { t.hover } else { t.input };
        let arrow_box = egui::epaint::RectShape::new(under, ar_radius, fill, Stroke::new(ARROW_BORDER, t.input_border), egui::StrokeKind::Inside);
        ui.painter().set(arrow_bg, arrow_box);
        // Not while typing: the family list drops from the field's bottom edge and the hint would cover it.
        // A field only focused by Tab, with nothing typed, still gets its (delayed) keyboard hint.
        let typing = focused && text != current;
        if !tip.is_empty() && !typing && !s.open {
            show_hint(&resp, tip, &[]);
            show_hint(&aresp, tip, &[]);
        }
        icons::paint(ui.painter(), Rect::from_center_size(ar.center(), vec2(DROPDOWN, DROPDOWN)), "dropdown", t.icon, t.accent);
        // The list drops from the box's left edge, as in Word, not from the arrow (#535).
        // The font list closes on a chosen entry or a click outside it, not on its headings.
        let close = if search { egui::PopupCloseBehavior::CloseOnClickOutside } else { egui::PopupCloseBehavior::CloseOnClick };
        let mut menu_field = aresp.clone();
        menu_field.rect = resp.rect.union(ar);
        let menu = craft_ui::combo::Menu::new(ui, &menu_field).close_behavior(close).height(420.0);
        let row_h = if preview.is_some() { COMBO_PREVIEW_ROW_H } else { ui.spacing().interact_size.y };
        let item = |k: usize| items.get(k);
        let row = |k: usize| section_row(k, recent, &item);
        let total = section_rows(recent.len(), items.len());
        let list = |ui: &mut Ui, range: std::ops::Range<usize>, ()| {
            draw_rows(ui, RowDrawOptions { width, current, range, row: &row, highlight: None, preview, row_height: row_h })
        };
        let footer = |ui: &mut Ui| {
            let mut picked = None;
            ui.separator();
            if ui.button(tl!("Add Font…")).clicked() {
                picked = Some(ADD_FONT.to_string());
                ui.close();
            }
            #[cfg(target_arch = "wasm32")]
            if ui.button(tl!("Load System Installed Fonts…")).clicked() {
                picked = Some(LOAD_SYSTEM_FONTS.to_string());
                ui.close();
            }
            picked
        };
        let picked = if search {
            menu.show_rows_with_header_footer(|_| (), row_h, |()| total, list, footer).map(|(v, f)| v.or(f))
        } else {
            menu.show_rows_with_header(|_| (), row_h, |()| total, list)
        };
        if let Some(v) = picked.flatten() {
            out = Some(v);
        }
        let n = section_rows(recent_shown.len(), s.matches.len());
        if s.open && n > 0 {
            let matches = s.matches.clone();
            let scroll = reveal.then_some((s.offset, s.view_h));
            let lit = highlight_row(s.highlight, recent_shown.len());
            // A list of a new length is sized again (an area keeps its last size, so a list that
            // once held one match stayed one row tall). That pass draws nothing, so it is
            // discarded and the frame drawn again, as egui does for its own sizing passes.
            let list_id = search_id.with("list");
            let resize = n != s.rows;
            if resize || egui::AreaState::load(ui.ctx(), list_id).is_none() {
                ui.ctx().request_discard("font list resized");
            }
            s.rows = n;
            let area = egui::Area::new(list_id).order(egui::Order::Tooltip).fixed_pos(resp.rect.left_bottom()).sizing_pass(resize);
            let shown = area.show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        let item = |k: usize| matches.get(k).and_then(|&i| items.get(i));
                        let row = |k: usize| section_row(k, recent_shown, &item);
                        list_rows(ui, RowListOptions { width, current, n, row: &row, highlight: Some(lit), scroll, preview })
                    })
                    .inner
            });
            s.popup = Some(shown.response.rect);
            let (picked, offset, view_h) = shown.inner;
            (s.offset, s.view_h) = (offset, view_h);
            if let Some(v) = picked {
                out = Some(v);
                s.open = false;
                regain_focus = false;
                ui.memory_mut(|m| m.surrender_focus(te_id));
            }
        } else {
            s.popup = None;
            s.rows = 0;
        }
        if regain_focus && s.open {
            ui.memory_mut(|m| m.request_focus(te_id));
        }
    });
    let keep = (focused || s.open) && out.is_none();
    ui.data_mut(|d| {
        if keep {
            d.insert_temp(edit_id, text);
        } else {
            d.remove::<String>(edit_id);
        }
        if search {
            d.insert_temp(search_id, s);
        }
    });
    out
}

/// Height of a row drawn by a combo's preview closure (`Previews::font_preview_fn`'s entries).
pub const COMBO_PREVIEW_ROW_H: f32 = 24.0;

/// A combo's scrolling list. Only the rows in view are built: the font menu lists every installed
/// family and each preview loads that font, so building all rows loaded every font on the system
/// (gigabytes on large font collections) and froze the app (#121).
pub fn combo_list(ui: &mut Ui, width: f32, current: &str, items: &[String], preview: Option<&dyn Fn(&mut Ui, &str) -> Response>) -> Option<String> {
    list_rows(ui, RowListOptions { width, current, n: items.len(), row: &|k| items.get(k).map(Row::Item), highlight: None, scroll: None, preview }).0
}

/// A row of a combo's list: a dim section heading nothing picks, or an entry.
pub(super) enum Row<'a> {
    Heading(&'a str),
    Item(&'a String),
}

/// Rows in a list of `n` entries headed by `recent` recent ones: the two headings and the recent
/// entries come on top when there are any.
pub(super) fn section_rows(recent: usize, n: usize) -> usize {
    if recent == 0 { n } else { n.saturating_add(recent).saturating_add(2) }
}

/// Row `k` of a list headed by the `recent` families: Recently Used Fonts, those families, All
/// Fonts, then `item(0..)`. Without recent families the rows are the items.
pub(super) fn section_row<'a>(k: usize, recent: &'a [String], item: &dyn Fn(usize) -> Option<&'a String>) -> Option<Row<'a>> {
    if recent.is_empty() {
        return item(k).map(Row::Item);
    }
    if k == 0 {
        return Some(Row::Heading(tl!("Recently Used Fonts")));
    }
    if let Some(r) = recent.get(k.saturating_sub(1)) {
        return Some(Row::Item(r));
    }
    if k == recent.len().saturating_add(1) {
        return Some(Row::Heading(tl!("All Fonts")));
    }
    item(k.saturating_sub(recent.len()).saturating_sub(2)).map(Row::Item)
}

/// The row of entry `entry` in a list headed by `recent` recent families ([`section_row`]).
pub(super) fn highlight_row(entry: usize, recent: usize) -> usize {
    if recent == 0 {
        entry
    } else if entry < recent {
        entry.saturating_add(1)
    } else {
        entry.saturating_add(2)
    }
}

/// A section heading's row of `size`: dim text, a rule across the list above it after the
/// first row, nothing to click. The row asks for no more than `size`: the list's width is its
/// entries' (a sizing pass offers an unbounded width).
fn heading_row(ui: &mut Ui, text: &str, size: egui::Vec2, rule: bool) {
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.noninteractive.bg_stroke;
    if rule {
        ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), line);
    }
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add_space(6.0);
        ui.add(egui::Label::new(egui::RichText::new(text).small().color(weak)).selectable(false));
    });
}

/// The rows `range` of a combo's list, in `ui`'s scroll area. Returns the clicked item.
struct RowDrawOptions<'rows, 'text> {
    width: f32,
    current: &'text str,
    range: std::ops::Range<usize>,
    row: &'rows dyn Fn(usize) -> Option<Row<'text>>,
    highlight: Option<usize>,
    preview: Option<&'rows dyn Fn(&mut Ui, &str) -> Response>,
    row_height: f32,
}

fn draw_rows(ui: &mut Ui, options: RowDrawOptions<'_, '_>) -> Option<String> {
    let RowDrawOptions { width, current, range, row, highlight, preview, row_height: row_h } = options;
    let mut out = None;
    let fill = Tokens::get(ui.ctx()).checked;
    ui.set_min_width(width + 60.0);
    for k in range {
        let it = match row(k) {
            Some(Row::Item(it)) => it,
            Some(Row::Heading(h)) => {
                heading_row(ui, h, vec2(width + 60.0, row_h), k > 0);
                continue;
            }
            None => continue,
        };
        let lit = highlight == Some(k);
        let clicked = match preview {
            Some(p) => {
                if lit {
                    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), row_h));
                    ui.painter().rect_filled(r, 3.0, fill);
                }
                p(ui, it).clicked()
            }
            None => ui.selectable_label(if highlight.is_some() { lit } else { it == current }, it.as_str()).clicked(),
        };
        if clicked {
            out = Some(it.clone());
        }
    }
    out
}

/// The rows of [`combo_list`], virtualised, for `n` rows fetched by index. With a `highlight`
/// that row is marked instead of `current`; `scroll` (last frame's offset and visible height)
/// scrolls the highlight into view. Returns the clicked item, the scroll offset and the height.
struct RowListOptions<'rows, 'text> {
    width: f32,
    current: &'text str,
    n: usize,
    row: &'rows dyn Fn(usize) -> Option<Row<'text>>,
    highlight: Option<usize>,
    scroll: Option<(f32, f32)>,
    preview: Option<&'rows dyn Fn(&mut Ui, &str) -> Response>,
}

fn list_rows(ui: &mut Ui, options: RowListOptions<'_, '_>) -> (Option<String>, f32, f32) {
    let RowListOptions { width, current, n, row, highlight, scroll, preview } = options;
    let mut out = None;
    let row_h = if preview.is_some() { COMBO_PREVIEW_ROW_H } else { ui.spacing().interact_size.y };
    let desired_h = (n as f32 * (row_h + ui.spacing().item_spacing.y)).min(420.0);
    ui.set_min_height(desired_h);
    let mut area = egui::ScrollArea::vertical().max_height(desired_h);
    if let (Some(h), Some((offset, view_h))) = (highlight, scroll) {
        let top = h as f32 * (row_h + ui.spacing().item_spacing.y);
        let bottom = top + row_h;
        let offset = if view_h <= 0.0 {
            // No list was visible for the previous query. Start at the first row, including
            // its heading, rather than scrolling to the first selectable entry below it.
            0.0
        } else if top < offset {
            top
        } else if bottom > offset + view_h {
            bottom - view_h
        } else {
            offset
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    let shown = area.show_rows(ui, row_h, n, |ui, range| {
        if let Some(v) = draw_rows(ui, RowDrawOptions { width, current, range, row, highlight, preview, row_height: row_h }) {
            out = Some(v);
        }
    });
    (out, shown.state.clamped_offset().y, shown.inner_rect.height())
}

/// How a drawn button looks this frame. Hover fades in over egui's animation time and the
/// pressed state blends in while the pointer is down, as on egui's own buttons. Framed buttons
/// take egui's widget visuals (fill, stroke, text colour, expansion) so they match `ui.button`;
/// frameless ones take the ribbon's hover/pressed tokens, like [`small`] and [`big`]. With
/// animation effects off (egui's animation time is zero) both states show at once.
pub struct DrawnState {
    pub fill: Color32,
    pub stroke: Stroke,
    pub text: Color32,
    pub expansion: f32,
}

pub fn drawn_state(ui: &Ui, resp: &Response, framed: bool) -> DrawnState {
    let pressed = resp.is_pointer_button_down_on();
    let h = ui.ctx().animate_bool_responsive(resp.id.with("hover"), resp.hovered() || pressed);
    let p = ui.ctx().animate_bool_with_time(resp.id.with("press"), pressed, ui.style().animation_time.min(0.06));
    if framed {
        let w = &ui.visuals().widgets;
        let mix = |a: Color32, b: Color32, c: Color32| a.lerp_to_gamma(b, h).lerp_to_gamma(c, p);
        let num = |a: f32, b: f32, c: f32| egui::lerp(egui::lerp(a..=b, h)..=c, p);
        DrawnState {
            fill: mix(w.inactive.weak_bg_fill, w.hovered.weak_bg_fill, w.active.weak_bg_fill),
            stroke: Stroke::new(
                num(w.inactive.bg_stroke.width, w.hovered.bg_stroke.width, w.active.bg_stroke.width),
                mix(w.inactive.bg_stroke.color, w.hovered.bg_stroke.color, w.active.bg_stroke.color),
            ),
            text: mix(w.inactive.fg_stroke.color, w.hovered.fg_stroke.color, w.active.fg_stroke.color),
            expansion: num(w.inactive.expansion, w.hovered.expansion, w.active.expansion),
        }
    } else {
        let t = Tokens::get(ui.ctx());
        DrawnState {
            fill: Color32::TRANSPARENT.lerp_to_gamma(t.hover, h).lerp_to_gamma(t.pressed, p),
            stroke: Stroke::NONE,
            text: t.text,
            expansion: 0.0,
        }
    }
}
