use super::NavTreeStyle;
use super::{Counters, Document, LevelSuffix, NumFormat, Path, Pos, StoryRef, Stroke, Tokens, Ui, WordApp, json, regular, semibold, vec2};

/// Modelo 2 — Curva Orgânica Fluida, options A and B, from the reference SVG's coordinates: the
/// node of level 0 sits at x=18, a 16 px step per level (level 1 at 34, level 2 at 50), the badge
/// of a parent 12 px after its node, a 14 px icon square and a 6 px gap after it. The elbow is a
/// quadratic of radius 8 (the SVG's Q 18 96 26 96 L 30 96).
/// +25% thickness applied for enhanced finish and high-DPI clarity.
const NAV_TRUNK_WIDTH: f32 = 1.6;
const NAV_TRUNK_ACTIVE_WIDTH: f32 = 2.0;
const NAV_ROW_HEIGHT: f32 = 26.0;
/// From the right of the node slot (or the indent, for a leaf) to the keycap badge/label.
const NAV_CONTENT_GAP: f32 = 6.0;
/// The mockup's keycap badge: a 14 px square.
const NAV_ICON_BOX: f32 = 14.0;
/// The colour emoji's box: the SVG's emoji placeholders are 11–13 px, and 13 keeps the following
/// text on the SVG's x=67 column (the title's own space after the emoji supplies the 6 px gap).
const NAV_EMOJI_BOX: f32 = 13.0;
/// The selection pill starts this far left of the content, where the active curve ends.
const NAV_PILL_INSET: f32 = 2.0;
/// The node column of level 0 and the step between levels (SVG: 18, 34, 50…).
const NAV_NODE_X0: f32 = 18.0;
const NAV_COLUMN_STEP: f32 = 16.0;
/// A parent's badge sits this far after its node (SVG: node 34, badge 46).
const NAV_PARENT_BADGE_DX: f32 = 12.0;
/// Radius of the curved elbow in the reference SVG (8 px).
const NAV_CURVE_RADIUS: f32 = 8.0;
/// The pane body's own left padding (frame + scroll area): the SVG's coordinates start at the
/// pane's edge, so the rows are drawn this much to the left to land on the same columns.
const NAV_PANE_TRIM: f32 = 18.5;

/// Radius of a parent's circular chevron node (7 px at level 0, 6.5 px below, as the mockup).
fn nav_node_radius(level: u8) -> f32 {
    if level == 0 { 7.0 } else { 6.5 }
}

/// Radius of the curved elbow (the reference SVG's 8 px).
fn nav_curve_radius(_level: u8) -> f32 {
    NAV_CURVE_RADIUS
}

/// The X of the trunk (and of the node) of a level: 18, 34, 50…
fn nav_node_x(level: u8) -> f32 {
    NAV_NODE_X0 + level as f32 * NAV_COLUMN_STEP
}

/// The badge/icon column of a row: a parent's badge follows its node by 12; a leaf starts at its
/// level's column.
fn nav_content_x(level: u8) -> f32 {
    nav_node_x(level) + NAV_PARENT_BADGE_DX
}

/// One trunk segment painted on a row.
#[derive(Clone, Debug, PartialEq)]
struct NavTrunk {
    x: f32,
    top: f32,
    bottom: f32,
    active: bool,
}

/// The curved elbow into the row: a quadratic Bézier from the trunk to the horizontal, then the
/// straight part to the content.
#[derive(Clone, Debug, PartialEq)]
struct NavCurve {
    start: egui::Pos2,
    control: egui::Pos2,
    end: egui::Pos2,
    horizontal_end: f32,
    active: bool,
}

/// A parent row's circular chevron node with 3D finish.
#[derive(Clone, Debug, PartialEq)]
struct NavNode {
    centre: egui::Pos2,
    radius: f32,
    expanded: bool,
    active: bool,
}

/// Everything one row paints, computed without a GPU so the geometry can be tested.
#[derive(Clone, Debug, Default, PartialEq)]
struct NavGeometry {
    content_x: f32,
    pill: Option<egui::Rect>,
    trunks: Vec<NavTrunk>,
    curves: Vec<NavCurve>,
    node: Option<NavNode>,
}

/// The geometry of one heading row (pure): trunks, the curved elbow, the chevron node and the
/// selection pill. `style` decides whether an inactive leaf gets its own elbow (B) or not (A).
/// `has_root_above` and `has_root_below` ensure the root rail (level 0) connects consecutive roots.
#[allow(clippy::too_many_arguments)]
fn nav_row_geometry(
    row: egui::Rect,
    level: u8,
    guides: &[u8],
    last: bool,
    active: bool,
    active_guides: &[u8],
    style: NavTreeStyle,
    collapsed: Option<bool>,
    selected: bool,
    has_root_above: bool,
    has_root_below: bool,
) -> NavGeometry {
    let mid = row.center().y;
    let left = row.left() - NAV_PANE_TRIM;
    let radius = nav_node_radius(level);
    let content_x = left + nav_content_x(level);
    let mut g = NavGeometry { content_x, ..Default::default() };
    if selected {
        // SVG: pill x=48 for content x=50, height 24, right edge 13 px from the pane's.
        let h = 24.0;
        let top = mid - h * 0.5;
        g.pill = Some(egui::Rect::from_min_max(egui::pos2(content_x - NAV_PILL_INSET, top), egui::pos2(row.right() - 13.0, top + h)));
    }
    // A parent's circular chevron node sits on its own column's trunk.
    // If it has a guide at its own level running from above, or if it is a root with another root above,
    // the trunk enters from above.
    // Only if expanded (or if it is a root with another root below) does a trunk leave its base downwards.
    if let Some(expanded) = collapsed.map(|c| !c) {
        let centre = egui::pos2(left + nav_node_x(level), mid);
        let stroke_active = active;
        if guides.contains(&level) || (level == 0 && has_root_above) {
            g.trunks.push(NavTrunk { x: centre.x, top: row.top(), bottom: centre.y - radius, active: stroke_active });
        }
        if expanded || (level == 0 && has_root_below) {
            g.trunks.push(NavTrunk { x: centre.x, top: centre.y + radius, bottom: row.bottom(), active: stroke_active });
        }
        g.node = Some(NavNode { centre, radius, expanded, active });
    }
    // Trunks of every ancestor level; the innermost one ends at the curve when it is the last
    // child (no loose end below it) and the row has an elbow.
    // The outer root rail (level 0) continues through to the bottom as long as another root exists below.
    if let Some(innermost) = guides.last().copied() {
        let curve = nav_curve_radius(innermost);
        let draw_curve = style == NavTreeStyle::FleetConnected || active;
        for &a in guides {
            if a == level && g.node.is_some() {
                continue; // Handled directly above on the node
            }
            let lit = active_guides.contains(&a);
            let bottom = if last && a == innermost && draw_curve && (a > 0 || !has_root_below) { mid - curve } else { row.bottom() };
            g.trunks.push(NavTrunk { x: left + nav_node_x(a), top: row.top(), bottom, active: lit });
        }
        if draw_curve {
            let x = left + nav_node_x(innermost);
            // A parent's elbow reaches the circle edge; a leaf's elbow reaches the selection pill
            // or connects directly to its badge/emoji without gaps.
            let horizontal_end = match &g.node {
                Some(node) => node.centre.x - node.radius,
                None if selected => content_x - NAV_PILL_INSET,
                None => content_x,
            };
            if horizontal_end > x {
                g.curves.push(NavCurve {
                    start: egui::pos2(x, mid - curve),
                    control: egui::pos2(x, mid),
                    end: egui::pos2(x + curve, mid),
                    horizontal_end,
                    active: active || active_guides.contains(&innermost),
                });
            }
        }
    }
    g
}

/// The child rows of a level under a node keep the trunk continuous, so the node's lower stub and
/// the first child's trunk meet without a gap.
#[cfg(test)]
fn trunk_has_no_gap(a: &NavTrunk, b: &NavTrunk) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.bottom - b.top).abs() < 0.01
}

/// The row indices on the caret's branch, from the root down to the selected heading.
fn active_chain(levels: &[u8], visible: &[bool], current: Option<usize>) -> Vec<usize> {
    let mut chain = Vec::new();
    let Some(mut i) = current else { return chain };
    if !visible.get(i).copied().unwrap_or(false) {
        return chain;
    }
    chain.push(i);
    let mut level = levels[i];
    loop {
        let parent = (0..i).rev().find(|&j| visible.get(j).copied().unwrap_or(false) && levels[j] < level);
        let Some(j) = parent else { break };
        chain.push(j);
        i = j;
        level = levels[j];
    }
    chain.reverse();
    chain
}

/// The guide levels of row `i` whose trunk belongs to the active branch: an ancestor level is lit
/// when its parent row is on the chain and the row lies between that parent and the selected row,
/// which carries the light through the siblings passed on the way down.
fn branch_guide_levels(ancestors: &[u8], chain: &[usize], levels: &[u8], i: usize, current: Option<usize>) -> Vec<u8> {
    let Some(current) = current else { return Vec::new() };
    ancestors.iter().copied().filter(|a| chain.iter().any(|&k| levels.get(k) == Some(a) && k <= i) && i <= current).collect()
}

/// A heading that starts with a keycap (`1️⃣`, `#️⃣`): the digit and the text after the space
/// that follows it. The interface fonts have no keycap ligature (they would draw the digit and
/// drop the enclosing mark), so the pane draws the keycap as the badge the page shows.
fn keycap_prefix(text: &str) -> Option<(char, &str)> {
    let mut it = text.char_indices();
    let (_, digit) = it.next()?;
    if !(digit.is_ascii_digit() || digit == '#' || digit == '*') {
        return None;
    }
    for (i, c) in it {
        match c {
            // The variation selector is optional (a keycap may be digit + enclosing keycap).
            '\u{FE0E}' | '\u{FE0F}' => {}
            '\u{20E3}' => {
                let end = i + c.len_utf8();
                let rest = text.get(end..).unwrap_or("");
                return Some((digit, rest.strip_prefix(' ').unwrap_or(rest)));
            }
            _ => return None,
        }
    }
    None
}

/// Byte ranges of the emoji sequences in a heading's text, in order.
fn emoji_clusters(text: &str) -> Vec<(usize, usize)> {
    let starts = wordcraft_doc::cluster::cluster_starts(text);
    let mut out = Vec::new();
    for (i, &s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(text.len());
        if let Some(c) = text.get(s..e) {
            // Presentation sequences (`⚖️`, keycaps, ZWJ) and characters that are emoji by
            // themselves (`📘`): the pane paints both from the colour emoji font.
            let chars: Vec<char> = c.chars().collect();
            if wordcraft_fonts::emoji_sequence(&chars) || chars.first().is_some_and(|&f| wordcraft_fonts::is_emoji(f)) {
                out.push((s, e));
            }
        }
    }
    out
}

/// The interface's colour image for an emoji, kept between frames (rasterising it again every
/// frame would burn the CPU for nothing: the pane redraws while typing).
fn emoji_texture(ui: &Ui, text: &str, px: u32) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique(("nav_emoji", text, px));
    if let Some(t) = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let img = wordcraft_render::emoji_image(text, px)?;
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba);
    let tex = ui.ctx().load_texture(format!("nav-emoji-{text}"), color, egui::TextureOptions::LINEAR);
    ui.ctx().data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// Paint the colour images of the emoji in `text` over the glyphs the pane's (transparent) text
/// found for them. The interface fonts have emoji only in monochrome; the page draws them from the
/// colour font, and the Navigator follows the page rather than showing grey shapes.
fn paint_emoji(ui: &Ui, painter: &egui::Painter, pos: egui::Pos2, galley: &egui::Galley, text: &str) {
    let ppp = ui.ctx().pixels_per_point();
    let mut glyphs = galley.rows.iter().flat_map(|r| r.glyphs.iter());
    for (s, e) in emoji_clusters(text) {
        let Some(first) = text.get(s..e).and_then(|c| c.chars().next()) else { continue };
        // One glyph per drawn character, in order: the cluster's first character finds its box.
        let Some(g) = glyphs.find(|g| g.chr == first) else { continue };
        // One icon column, as in the mockup: every emoji fits a 14 px box (height) and can never
        // be wider than its slot (advance + the explicit gap), so a wide emoji neither swells nor
        // touches the following letter; the dominant dimension is capped, so sizes match.
        let box_side = NAV_EMOJI_BOX.max(1.0);
        // The image fills only the glyph's advance; the explicit letter gap after the placeholder
        // is what separates it from the next letter, as the mockup's 6 px icon gap does.
        let slot_w = g.advance_width.max(6.0);
        let Some(tex) = emoji_texture(ui, &text[s..e], (box_side * ppp).round().max(1.0) as u32) else { continue };
        let size = tex.size_vec2();
        let aspect = (size.x / size.y.max(1.0)).clamp(0.2, 5.0);
        let mut h = box_side;
        let mut w = h * aspect;
        if w > slot_w {
            w = slot_w;
            h = w / aspect;
        }
        let top = (pos.y + (galley.size().y - h) * 0.5).max(pos.y);
        let rect = egui::Rect::from_min_size(egui::pos2(pos.x + g.pos.x, top), vec2(w, h));
        painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

/// The segmented `[ Headings | Pages | Results ]` control: one line, three equal columns, a
/// dark container with a hairline border and a filled blue active tab (the mockup's style).
fn nav_tabs(ui: &mut Ui, current: &mut String) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(120.0);
    let height = 24.0;
    let (outer, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(outer, 6.0, t.nav_tab_bg);
    ui.painter().rect_stroke(outer, 6.0, Stroke::new(1.0, t.nav_tab_border), egui::StrokeKind::Inside);
    let cell = width / 3.0;
    for (i, (key, label)) in [("headings", "Headings"), ("pages", "Pages"), ("results", "Results")].into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(outer.left() + i as f32 * cell, outer.top()), vec2(cell, height));
        let resp = ui.interact(rect, ui.scope_id().with(("nav_tab", key)), egui::Sense::click());
        let on = current.as_str() == key;
        if on {
            ui.painter().rect_filled(rect.shrink(2.0), 5.0, t.nav_tab_active);
        }
        let colour = if on { t.nav_tab_active_text } else { ui.visuals().text_color() };
        let font = if on { semibold(11.5) } else { regular(11.5) };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tl!(label), font, colour);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tl!(label)));
        if resp.clicked() {
            *current = key.to_string();
        }
    }
}

/// Paint the computed geometry of a row: trunks (with the active branch's glow), the quadratic
/// elbow, and the circular chevron node.
fn paint_nav_geometry(painter: &egui::Painter, g: &NavGeometry, t: &Tokens) {
    for trunk in &g.trunks {
        let stroke = if trunk.active { Stroke::new(NAV_TRUNK_ACTIVE_WIDTH, t.nav_trunk_active) } else { Stroke::new(NAV_TRUNK_WIDTH, t.nav_trunk) };
        if trunk.active {
            let glow = Stroke::new(NAV_TRUNK_ACTIVE_WIDTH + 2.0, t.nav_trunk_active.gamma_multiply(0.35));
            painter.vline(trunk.x, trunk.top..=trunk.bottom, glow);
        }
        painter.vline(trunk.x, trunk.top..=trunk.bottom, stroke);
    }
    for curve in &g.curves {
        let stroke = if curve.active { Stroke::new(NAV_TRUNK_ACTIVE_WIDTH, t.nav_curve_active) } else { Stroke::new(NAV_TRUNK_WIDTH, t.nav_curve) };
        if curve.active {
            painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                [curve.start, curve.control, curve.end],
                false,
                egui::Color32::TRANSPARENT,
                Stroke::new(NAV_TRUNK_ACTIVE_WIDTH + 2.0, t.nav_curve_active.gamma_multiply(0.35)),
            ));
            painter.hline(
                curve.end.x..=curve.horizontal_end,
                curve.end.y,
                Stroke::new(NAV_TRUNK_ACTIVE_WIDTH + 2.0, t.nav_curve_active.gamma_multiply(0.35)),
            );
        }
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [curve.start, curve.control, curve.end],
            false,
            egui::Color32::TRANSPARENT,
            stroke,
        ));
        if curve.horizontal_end > curve.end.x {
            painter.hline(curve.end.x..=curve.horizontal_end, curve.end.y, stroke);
        }
    }
    if let Some(node) = &g.node {
        draw_expand_button(painter, node, t);
    }
}

/// Draw a circular expand/collapse button with subtle 3D depth and a crisp vector chevron.
fn draw_expand_button(painter: &egui::Painter, node: &NavNode, t: &Tokens) {
    let (fill, border) = if node.active { (t.nav_node_fill_active, t.nav_node_border_active) } else { (t.nav_node_fill, t.nav_node_border) };
    painter.circle_filled(node.centre, node.radius, fill);
    painter.circle_stroke(node.centre, node.radius, Stroke::new(if node.active { 1.35 } else { 1.2 }, border));

    // Subtle 3D depth: top crescent reflection and bottom shadow
    let highlight = Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, if node.active { 75 } else { 40 }));
    let shadow = Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 75));
    painter.line_segment(
        [
            egui::pos2(node.centre.x - node.radius * 0.55, node.centre.y - node.radius * 0.45),
            egui::pos2(node.centre.x + node.radius * 0.55, node.centre.y - node.radius * 0.45),
        ],
        highlight,
    );
    painter.line_segment(
        [
            egui::pos2(node.centre.x - node.radius * 0.5, node.centre.y + node.radius * 0.6),
            egui::pos2(node.centre.x + node.radius * 0.5, node.centre.y + node.radius * 0.6),
        ],
        shadow,
    );

    // Chevron vetorial nítido (linha contínua conectada)
    let c = node.centre;
    let stroke = Stroke::new(1.5, egui::Color32::WHITE);
    let pts = if node.expanded {
        // Apontando para baixo (v)
        [egui::pos2(c.x - 3.2, c.y - 1.2), egui::pos2(c.x, c.y + 2.0), egui::pos2(c.x + 3.2, c.y - 1.2)]
    } else {
        // Apontando para a direita (>)
        [egui::pos2(c.x - 1.2, c.y - 3.2), egui::pos2(c.x + 2.0, c.y), egui::pos2(c.x - 1.2, c.y + 3.2)]
    };
    painter.line_segment([pts[0], pts[1]], stroke);
    painter.line_segment([pts[1], pts[2]], stroke);
}

/// One heading in the pane: the trunk/elbow/node geometry, the keycap badge, the emoji and the
/// label. `active` is the caret's branch (and the selected row); `style` switches between A and B.
#[allow(clippy::too_many_arguments)]
pub(super) fn nav_heading(
    ui: &mut Ui,
    id: egui::Id,
    level: u8,
    text: &str,
    selected: bool,
    collapsed: Option<bool>,
    guides: &[u8],
    last: bool,
    active: bool,
    active_guides: &[u8],
    style: NavTreeStyle,
    has_root_above: bool,
    has_root_below: bool,
) -> (bool, egui::Response) {
    let t = Tokens::get(ui.ctx());
    let row_h = NAV_ROW_HEIGHT;
    let keycap = keycap_prefix(text);
    let label_text = keycap.map_or(text, |(_, rest)| rest);
    let radius = nav_node_radius(level);
    let has_children = collapsed.is_some();
    let row_w = ui.available_width().max(0.0);
    let content_x = nav_content_x(level);
    let badge_w = keycap.map_or(0.0, |_| NAV_ICON_BOX + NAV_CONTENT_GAP);
    // The selection pill insets its content by 5 px (SVG: pill x=48, icon x=55).
    let shift = if selected { 5.0 } else { 0.0 };
    let label_x = content_x + badge_w + shift;
    let gap = ui.spacing().item_spacing.x;
    let label_w = (row_w - label_x - 2.0 * gap).max(24.0);

    let (row, _) = ui.allocate_exact_size(vec2(row_w, row_h), egui::Sense::hover());
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let geometry = nav_row_geometry(row, level, guides, last, active, active_guides, style, collapsed, selected, has_root_above, has_root_below);
    let painter = ui.painter().with_clip_rect(ui.clip_rect().expand2(vec2(NAV_PANE_TRIM, 0.0)));
    paint_nav_geometry(&painter, &geometry, &t);
    if let Some(pill) = geometry.pill {
        painter.rect_filled(pill, 6.0, t.nav_pill_fill);
        painter.rect_stroke(pill, 6.0, Stroke::new(1.0, t.nav_pill_border), egui::StrokeKind::Inside);
    } else if hovered {
        let hover = egui::Rect::from_min_max(
            egui::pos2(row.left() - NAV_PANE_TRIM + content_x - NAV_PILL_INSET, row.center().y - row_h * 0.5),
            egui::pos2(row.right() - 6.0, row.center().y + row_h * 0.5),
        );
        painter.rect_filled(hover, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }

    // The label: hierarchy, contrast, and emoji laid out transparent so their colour images sit on top.
    let font = if level <= 1 && (active || collapsed == Some(false)) {
        semibold(12.5)
    } else if selected {
        semibold(12.0)
    } else {
        regular(12.0)
    };
    let text_color = if selected {
        egui::Color32::WHITE
    } else if level <= 1 {
        egui::Color32::from_rgb(0xF8, 0xFA, 0xFC)
    } else {
        egui::Color32::from_rgb(0xD1, 0xD5, 0xDB)
    };
    let ink = egui::TextFormat { font_id: font.clone(), color: text_color, ..Default::default() };
    // The emoji placeholder keeps the regular metrics (bold would change its advance and crowd
    // the next letter), plus an explicit gap that the image can use without reaching the text.
    let clear = egui::TextFormat { font_id: regular(12.0), color: egui::Color32::TRANSPARENT, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut prev = 0;
    for (s, e) in emoji_clusters(label_text) {
        if prev < s {
            job.append(&label_text[prev..s], 0.0, ink.clone());
        }
        job.append(&label_text[s..e], 0.0, clear.clone());
        prev = e;
    }
    if prev < label_text.len() {
        job.append(&label_text[prev..], 0.0, ink.clone());
    }
    job.wrap = egui::text::TextWrapping { max_width: label_w, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
    let galley = ui.painter().layout_job(job);

    let mut toggled = false;
    let label = ui
        .scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            // The SVG's columns are exact; any implicit spacing between the row's pieces would
            // shift every badge/icon column by a few points, so only the explicit gaps count.
            ui.spacing_mut().item_spacing.x = 0.0;
            // Every row reserves the same chevron gap, whether or not it has children: the button
            // must never push one sibling's title deeper than another's. A leaf leaves the slot
            // empty (the geometry only paints a node for parents).
            ui.add_space((nav_node_x(level) - radius).max(0.0));
            let (node_rect, _) = ui.allocate_exact_size(vec2(2.0 * radius, row_h), egui::Sense::hover());
            if has_children {
                let node_resp = ui.interact(node_rect.translate(vec2(-NAV_PANE_TRIM, 0.0)), id.with("node"), egui::Sense::click());
                if node_resp.clicked() {
                    toggled = true;
                }
                node_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, collapsed == Some(false), tl!("Expand or collapse")));
            }
            ui.add_space((content_x - (nav_node_x(level) + radius)).max(0.0));
            if selected {
                ui.add_space(shift);
            }
            if let Some((digit, _)) = keycap {
                let (slot, _) = ui.allocate_exact_size(vec2(NAV_ICON_BOX, row_h), egui::Sense::hover());
                let badge = egui::Rect::from_center_size(slot.center() + vec2(-NAV_PANE_TRIM, 0.0), vec2(NAV_ICON_BOX, NAV_ICON_BOX));
                let fill = if active || selected { t.nav_badge_active } else { t.nav_badge };
                painter.rect_filled(badge, 3.0, fill);
                painter.text(badge.center(), egui::Align2::CENTER_CENTER, digit, semibold(9.5), egui::Color32::WHITE);
                ui.add_space(NAV_CONTENT_GAP);
            }
            let width = ui.available_width().max(0.0);
            let (label_rect, resp) = ui.allocate_exact_size(vec2(width, row_h), egui::Sense::click());
            let pos = label_rect.left_top() + vec2(-NAV_PANE_TRIM, ((row_h - galley.size().y) * 0.5).max(0.0));
            painter.galley(pos, galley.clone(), ui.visuals().text_color());
            paint_emoji(ui, &painter, pos, &galley, label_text);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, text));
            ui.interact(label_rect.translate(vec2(-NAV_PANE_TRIM, 0.0)), id, egui::Sense::click())
        })
        .inner;

    if ui.ctx().memory(|m| m.has_focus(id)) {
        ui.painter().rect_stroke(row.shrink(1.0), 4.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    (toggled, label.on_hover_text(text))
}

/// Draws the Navigation pane: search box and tabs, page list, find results and heading outline.
pub(super) fn nav(app: &mut WordApp, ui: &mut Ui) {
    let qid = app.window_id("nav_query");
    let mut q = ui.data(|d| d.get_temp::<String>(qid)).unwrap_or_default();
    // Ctrl+F with text selected searches for it, as Word does (a selection within one line).
    let focus = std::mem::take(&mut app.nav_search_focus);
    let picked = focus.then(|| app.session.selected_text()).filter(|t| !t.is_empty() && !t.contains(['\n', '\r']) && t.chars().count() <= 255);
    if let Some(t) = &picked {
        q = t.clone();
    }
    let mut out = egui::TextEdit::singleline(&mut q).hint_text(tl!("Search document")).desired_width(f32::INFINITY).show(ui);
    let r = out.response.clone();
    if focus {
        // Focused with its text selected, so typing replaces it.
        r.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(q.chars().count()));
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), r.id);
    }
    if r.changed() || picked.is_some() {
        app.ui.nav_tab = "results".into();
        ui.data_mut(|d| d.insert_temp(qid, q.clone()));
        if !q.is_empty() {
            let _ = app.session.run("edit.find", &json!({"text": q}));
        } else {
            app.session.find.results.clear();
        }
    }
    ui.add_space(4.0);
    nav_tabs(ui, &mut app.ui.nav_tab);
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match app.ui.nav_tab.as_str() {
        "pages" => {
            let l = app.session.layout();
            for (i, p) in l.pages.iter().enumerate() {
                let label = crate::i18n::fmt(tl!("Page {page}"), &[("page", &p.number.to_string())]);
                if ui.selectable_label(app.session.page_hint == i, label).clicked() {
                    let _ = app.run("edit.goto", json!({"page": i + 1}));
                }
            }
        }
        "results" => {
            let results = app.session.find.results.clone();
            ui.label(egui::RichText::new(format!("{} results", results.len())).small().weak());
            for (a, b) in results.iter().take(500) {
                let Some(p) = app.session.doc.para_at(a) else { continue };
                let from = p.clamp(a.off.saturating_sub(30));
                let to = p.clamp((b.off + 40).min(p.len()));
                let pre = p.text.get(from..a.off).unwrap_or("");
                let hit = p.text.get(a.off..b.off).unwrap_or("");
                let post = p.text.get(b.off..to).unwrap_or("");
                let mut job = egui::text::LayoutJob::default();
                let fmt = |c| egui::TextFormat { font_id: regular(12.0), color: c, ..Default::default() };
                job.append(pre, 0.0, fmt(ui.visuals().text_color()));
                // The match is marked the way Find marks it on the page, per theme (#656).
                let mark = Tokens::get(ui.ctx()).find_highlight;
                job.append(hit, 0.0, egui::TextFormat { font_id: semibold(12.0), background: mark, color: ui.visuals().text_color(), ..Default::default() });
                job.append(post, 0.0, fmt(ui.visuals().text_color()));
                job.wrap.max_width = ui.available_width();
                if ui.add(egui::Button::new(job).frame(false)).clicked() {
                    let _ = app.run("select.range", json!({"anchor": a, "focus": b}));
                }
                ui.separator();
            }
        }
        _ => {
            // Modelo 2: the rows touch, so the vertical trunks have no gaps between them.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let caret = app.session.sel.focus.clone();
                let items = nav_headings(&app.session.doc);
                let current = items.iter().rposition(|(path, ..)| *path <= caret.path);
                if items.is_empty() {
                    ui.label(egui::RichText::new(tl!("Create an interactive outline of your document.\n\nIt's a great way to keep track of where you are or quickly move your content around.\n\nTo get started, go to the Home tab and apply Heading styles to the headings in your document.")).weak());
                }
                // A heading has children when the next heading is a deeper level (#1997).
                let has_children: Vec<bool> = items.iter().enumerate().map(|(i, (_, lv, _))| items.get(i + 1).is_some_and(|n| n.1 > *lv)).collect();
                let collapsed_flags: Vec<bool> = items.iter().enumerate().map(|(i, (p, _, _))| has_children[i] && app.ui.nav_collapsed.contains(p)).collect();
                let levels: Vec<u8> = items.iter().map(|(_, lv, _)| *lv).collect();
                let visible = visible_headings(&levels, &collapsed_flags);
                let chain = active_chain(&levels, &visible, current);
                let tree_style = app.ui.nav_tree_style;
                let current_index = current;
                // The levels this heading nests under, for the tree's connector lines.
                let mut ancestors: Vec<u8> = Vec::new();
                for (i, (path, lv, txt)) in items.iter().enumerate() {
                    if !visible[i] {
                        continue;
                    }
                    while ancestors.last().is_some_and(|a| *a >= *lv) {
                        ancestors.pop();
                    }
                    // The last child of its parent closes the guide line under it at the row's middle.
                    let last = items
                        .iter()
                        .enumerate()
                        .skip(i + 1)
                        .find(|(j, _)| visible[*j])
                        .is_none_or(|(_, (_, next, _))| *next < *lv);
                    let collapsed = collapsed_flags[i];
                    let row_id = ui.make_persistent_id(("nav_row", path));
                    // The trunks of the active branch: each ancestor level whose parent row is on the
                    // chain and which lies above (or at) the selected row.
                    let active_guides = branch_guide_levels(&ancestors, &chain, &levels, i, current_index);
                    let on_chain = chain.contains(&i);
                    let has_root_above = items.iter().enumerate().take(i).any(|(j, (_, l, _))| visible[j] && *l == 0);
                    let has_root_below = items.iter().enumerate().skip(i + 1).any(|(j, (_, l, _))| visible[j] && *l == 0);
                    let (toggle, r) = nav_heading(
                        ui,
                        row_id,
                        *lv,
                        txt,
                        current == Some(i),
                        has_children[i].then_some(collapsed),
                        &ancestors,
                        last,
                        on_chain,
                        &active_guides,
                        tree_style,
                        has_root_above,
                        has_root_below,
                    );
                    if toggle {
                        if !app.ui.nav_collapsed.remove(path) {
                            app.ui.nav_collapsed.insert(path.clone());
                        }
                    } else if r.clicked() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space))) {
                        let _ = app.run("caret.set", json!({"pos": Pos { story: StoryRef::Body, path: path.clone(), off: 0 }}));
                        // Align the chosen heading's page immediately against this pane, rather
                        // than the indeterminate minimum movement from scroll_to_rect.
                        app.canvas.nav_to_caret = true;
                        app.canvas.scroll_to_caret = false;
                        app.canvas.want_focus = true;
                    }
                    ancestors.push(*lv);
                }
            });
        }
    });
}

/// Which headings the pane shows: those under a collapsed heading (every following heading
/// of a deeper level) are hidden (#1997).
pub(super) fn visible_headings(levels: &[u8], collapsed: &[bool]) -> Vec<bool> {
    let mut hide_below: Option<u8> = None;
    levels
        .iter()
        .zip(collapsed)
        .map(|(&lv, &c)| {
            if hide_below.is_some_and(|h| lv > h) {
                return false;
            }
            hide_below = c.then_some(lv);
            true
        })
        .collect()
}

/// The body's headings (paragraphs with an outline level) in document order, as (path, level,
/// text). A numbered heading starts with the number it shows (`2.1 Method`), as in Word.
pub(super) fn nav_headings(doc: &Document) -> Vec<(Path, u8, String)> {
    // Every numbered paragraph advances the list counters, headings or not.
    let mut counters = Counters::default();
    let mut items = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let rp = doc.styles.resolve_para(&p.props);
        let number = p
            .props
            .numbering
            .or(rp.numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None));
        if let Some(lv) = rp.outline_level {
            let txt = p.plain_text().trim().to_string();
            if txt.is_empty() || rp.style.starts_with("TOC") {
                continue;
            }
            let txt = match number {
                Some((label, def)) if def.suffix == LevelSuffix::Nothing => format!("{label}{txt}"),
                Some((label, _)) => format!("{label} {txt}"),
                None => txt,
            };
            items.push((path, lv, txt));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Modelo 2: the elbow leaves the trunk going down (`mid - r`), turns through a quadratic
    /// control on the row's middle line and arrives horizontally from the left, never as `╭`.
    #[test]
    fn the_curve_starts_above_the_row_and_turns_right() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let g = nav_row_geometry(row, 2, &[0, 1], false, true, &[0, 1], NavTreeStyle::ZedLinear, None, false, false, false);
        let curve = g.curves.first().expect("an active row has its elbow");
        let mid = row.center().y;
        assert!((curve.start.y - (mid - nav_curve_radius(1))).abs() < 0.01, "starts above the middle");
        assert_eq!(curve.start.x, row.left() - NAV_PANE_TRIM + nav_node_x(1), "starts on the trunk");
        assert!(curve.control.y > curve.start.y && curve.control.y == mid, "the quadratic control is on the middle line");
        assert!(curve.end.x > curve.start.x);
        assert_eq!(curve.end.y, mid);
        assert_eq!(curve.horizontal_end, g.content_x, "a leaf's elbow connects directly to its badge");
    }

    /// The active curve ends exactly on the selection pill's left border.
    #[test]
    fn the_curve_meets_the_selection_pill() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let g = nav_row_geometry(row, 2, &[0, 1], false, true, &[0, 1], NavTreeStyle::ZedLinear, None, true, false, false);
        let pill = g.pill.expect("the selected row has a pill");
        let curve = g.curves.first().expect("the selected row has its elbow");
        assert!((pill.left() - curve.horizontal_end).abs() < 0.01, "{} vs {}", pill.left(), curve.horizontal_end);
        assert!((pill.left() - (g.content_x - NAV_PILL_INSET)).abs() < 0.01);
        assert!((22.0..=24.0).contains(&pill.height()), "pill height {}", pill.height());
    }

    /// Trunks of consecutive rows meet without a gap: the row above ends where the next begins.
    #[test]
    fn consecutive_rows_have_no_gap_in_the_trunk() {
        let a = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let b = a.translate(vec2(0.0, NAV_ROW_HEIGHT));
        let ga = nav_row_geometry(a, 2, &[0, 1], false, false, &[], NavTreeStyle::FleetConnected, None, false, false, false);
        let gb = nav_row_geometry(b, 2, &[0, 1], false, false, &[], NavTreeStyle::FleetConnected, None, false, false, false);
        let ta = ga.trunks.iter().find(|t| (t.x - (a.left() - NAV_PANE_TRIM + nav_node_x(0))).abs() < 0.01).expect("the outer trunk");
        let tb = gb.trunks.iter().find(|t| (t.x - (b.left() - NAV_PANE_TRIM + nav_node_x(0))).abs() < 0.01).expect("the outer trunk");
        assert!(trunk_has_no_gap(ta, tb));
    }

    /// A parent's rail goes through the node: it stops at the circle's top and resumes at its
    /// base, so the chevron is never crossed by a line.
    #[test]
    fn the_trunk_enters_and_leaves_the_node_without_crossing_it() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let g = nav_row_geometry(row, 1, &[0], true, true, &[0], NavTreeStyle::ZedLinear, Some(false), false, false, false);
        let node = g.node.expect("a parent has a node");
        assert!(g.trunks.iter().any(|t| (t.x - node.centre.x).abs() < 0.01 && t.bottom + 0.01 >= node.centre.y - node.radius));
        assert!(g.trunks.iter().any(|t| (t.x - node.centre.x).abs() < 0.01 && t.top <= node.centre.y + node.radius + 0.01));
        assert_eq!(node.radius, nav_node_radius(1));
        assert!(node.expanded);
    }

    /// Option A leaves inactive leaves without an elbow; Option B connects every child. Both keep
    /// the same trunk, node and pill geometry.
    #[test]
    fn option_a_has_no_inactive_elbows_and_option_b_connects_every_child() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let a = nav_row_geometry(row, 2, &[0, 1], false, false, &[], NavTreeStyle::ZedLinear, None, false, false, false);
        assert!(a.curves.is_empty(), "A: an inactive leaf has no comb teeth");
        let b = nav_row_geometry(row, 2, &[0, 1], false, false, &[], NavTreeStyle::FleetConnected, None, false, false, false);
        let curve = b.curves.first().expect("B: every child is connected");
        assert!(!curve.active, "an inactive child's elbow is grey");
        assert_eq!(b.pill, None);
    }

    /// B: a nested trunk ends on the last child's curve, while the outer (root) rail continues to
    /// the next root row.
    #[test]
    fn option_b_ends_the_nested_trunk_on_the_last_curve() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        let g = nav_row_geometry(row, 2, &[0, 1], true, false, &[], NavTreeStyle::FleetConnected, None, false, false, true);
        let inner = g.trunks.iter().find(|t| (t.x - (row.left() - NAV_PANE_TRIM + nav_node_x(1))).abs() < 0.01).expect("inner trunk");
        let outer = g.trunks.iter().find(|t| (t.x - (row.left() - NAV_PANE_TRIM + nav_node_x(0))).abs() < 0.01).expect("root rail");
        assert!((inner.bottom - (row.center().y - nav_curve_radius(1))).abs() < 0.01, "no loose end below the last child");
        assert!((outer.bottom - row.bottom()).abs() < 0.01, "the root rail goes on to the next root");
    }

    /// The root rail (level 0) connects consecutive roots: root 2 gets an incoming trunk from above
    /// and an outgoing trunk below, and children between roots keep the root rail continuous.
    #[test]
    fn root_rail_connects_consecutive_roots() {
        let row = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, NAV_ROW_HEIGHT));
        // Root 2 (has root 1 above, and root 3 below):
        let r2 = nav_row_geometry(row, 0, &[], false, false, &[], NavTreeStyle::FleetConnected, Some(false), false, true, true);
        let node = r2.node.expect("root 2 has a node");
        assert!(r2.trunks.iter().any(|t| (t.x - node.centre.x).abs() < 0.01 && t.bottom <= node.centre.y));
        assert!(r2.trunks.iter().any(|t| (t.x - node.centre.x).abs() < 0.01 && t.top >= node.centre.y));
    }

    /// The style is persisted as `navTreeStyle`; an old file (no key) or an unknown value loads as
    /// Option B (default) without dropping the rest of the preferences.
    #[test]
    fn the_saved_style_defaults_to_a_and_tolerates_unknown_values() {
        assert_eq!(NavTreeStyle::default(), NavTreeStyle::FleetConnected);
        assert_eq!(NavTreeStyle::from_code("fleet_connected"), Some(NavTreeStyle::FleetConnected));
        assert_eq!(NavTreeStyle::from_code("sepia"), None);
        let old: crate::UiState = serde_json::from_str(r#"{"tab":"Insert"}"#).unwrap();
        assert_eq!(old.nav_tree_style, NavTreeStyle::FleetConnected);
        let odd: crate::UiState = serde_json::from_str(r#"{"navTreeStyle":"sepia","tab":"Insert"}"#).unwrap();
        assert_eq!(odd.nav_tree_style, NavTreeStyle::FleetConnected);
        assert_eq!(odd.tab, "Insert");
    }
}
