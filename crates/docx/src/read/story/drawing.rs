use super::{
    AltText, Anchor, Arc, Blocks, CHARTEX_URI, ConnEnd, ContentTypes, El, Embedded, Float, FloatAlign, Glow, Graphic, GraphicItem, GraphicKind,
    INK_REL, INK_URI, InlineObject, MAX_STORY_DEPTH, Node, PartKind, Reader, Rels, Rgb, Shadow, ShapeEffects, ShapeExtra, ShapeKind, StoryCtx,
    TextWarp, Wrap, WrapText, cap_alt, children_of_choice, int, measure, on_off_attr, own_fill, preset_kind, rt, shape_paint, u32_of,
};

impl Reader<'_> {
    // ---- drawings ----

    /// Read a `w:drawing` frame and dispatch its payload to the picture, shape or graphic reader.
    /// Frame dimensions are converted from EMUs to bounded point values; placement, alternative
    /// text and drawing ids are carried into the object. Returns `None` when the frame or payload
    /// cannot be read. Ink written by WordCraft keeps its tool's caps rather than an outline cap.
    pub(super) fn read_drawing(&mut self, sc: &mut StoryCtx, d: &El, rels: &Rels) -> Option<InlineObject> {
        let (c, anchored) = match d.child("wp:inline") {
            Some(c) => (c, false),
            None => (d.child("wp:anchor")?, true),
        };
        let ext = c.child("wp:extent");
        let dim = |n: &str| ext.and_then(|e| e.attr(n)).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, crate::units::MAX_LEN_PT);
        let (w, h) = (dim("cx"), dim("cy"));
        let (alt, meta) = nv_alt(c.child("wp:docPr"));
        let mut float = if anchored { anchor_float(c) } else { Float::default() };
        float.alt = meta;
        float.hidden = c.child("wp:docPr").is_some_and(|p| on_off_attr(p, "hidden"));
        if let Some(e) = c.child("wp:effectExtent") {
            for (slot, n) in float.effect.iter_mut().zip(["l", "t", "r", "b"]) {
                // Negative for a rotated object narrower than its frame; settled below.
                *slot = e.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(-1584.0, 1584.0);
            }
        }
        let gd = c.child("a:graphic").and_then(|g| g.child("a:graphicData"))?;
        // Checked by URI before the blip search: a diagram's data can hold a blip further down.
        let uri = gd.attr("uri").unwrap_or("");
        if uri == INK_URI {
            let content = gd.child("w14:contentPart")?;
            let part = crate::read::part_of(rels, content.attr("r:id")?, INK_REL)?;
            let pkg = self.pkg;
            if self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part) != Some("application/inkml+xml") {
                return None;
            }
            let source = self.keep_embedded(c.child("a:graphic")?, rels, &[])?;
            float.set_spin(xfrm_spin(content.child("w14:xfrm")));
            let (px, py) = float.spin_pad(w, h);
            for (e, p) in float.effect.iter_mut().zip([px, py, px, py]) {
                if *e != 0.0 {
                    *e = (*e - p).clamp(0.0, 1584.0);
                }
            }
            // The renderer's existing missing-picture box marks ink without a usable preview.
            // InkML stays opaque: no approximation of its strokes or conversion to freeforms.
            let items = vec![GraphicItem::Image { rect: [0.0, 0.0, w, h], media: String::new() }];
            let graphic = Arc::new(Graphic { kind: GraphicKind::Ink, items, w, h, source: Some(source), ..Default::default() });
            return Some(InlineObject::Graphic { w, h, alt, float, graphic });
        }
        let graphic_kind = if uri.ends_with("/chart") {
            Some(GraphicKind::Chart)
        } else if uri.ends_with("/diagram") {
            Some(GraphicKind::Diagram)
        } else {
            None
        };
        if let Some(kind) = graphic_kind {
            // Kept so saving writes the chart or diagram back: the frame around it is written
            // from the object's (possibly moved or resized) size and position.
            let extra: Vec<String> = match kind {
                GraphicKind::Chart | GraphicKind::Ink => Vec::new(),
                GraphicKind::Diagram => self.diagram_drawing_rel(gd, rels).into_iter().collect(),
            };
            let source = c.child("a:graphic").and_then(|g| self.keep_embedded(g, rels, &extra));
            let graphic = self.graphic(kind, gd, rels, w, h, source);
            float.effect = float.effect.map(|e| e.max(0.0));
            return Some(InlineObject::Graphic { w, h, alt, float, graphic });
        }
        // A textbox may contain a picture; its descendant blip is not the outer payload.
        let direct_shape = gd.child("wps:wsp").filter(|shape| {
            // Picture fills still use the image fallback; the shape model has no image fill.
            shape.child("wps:spPr").and_then(|props| props.child("a:blipFill")).is_none()
        });
        let mut obj = if let Some(c) = gd.child("wpc:wpc") {
            self.read_canvas(sc, c, rels, w, h, float)
        } else if let Some(g) = gd.child("wpg:wgp") {
            self.read_group(sc, g, rels, w, h, float)?
        } else if direct_shape.is_none() && gd.find("a:blip").is_some() {
            self.read_pic(gd, rels, w, h, alt.clone(), float)?
        } else {
            let wsp = direct_shape.or_else(|| gd.find("wps:wsp"))?;
            let mut obj = self.read_wsp(sc, wsp, rels, w, h, float);
            // A freeform: whether it's ink WordCraft wrote.
            if let InlineObject::Shape { freeform: Some(f), extra, .. } = &mut obj
                && let Some(name) = c.child("wp:docPr").and_then(|p| p.attr("name"))
                && let Some(tool) = crate::read::freeform::ink_tool(name)
            {
                let f = Arc::make_mut(f);
                f.ink = Some(tool);
                f.order = crate::read::freeform::ink_order(name);
                // Ink's round ends belong to its tool, not the optional shape outline. Keep
                // its canonical model unchanged when reading the round cap the writer adds.
                extra.cap = Default::default();
            }
            obj
        };
        if !alt.is_empty() {
            obj.set_alt_text(&alt);
        }
        self.set_shape_id(&mut obj, c.child("wp:docPr").and_then(|p| p.attr("id")));
        // Word's effect extent also covers a rotated object's overhang; the model keeps that
        // apart (it follows from the angle), so take it back out.
        if let Some(f) = obj.float_mut() {
            let (px, py) = f.spin_pad(w, h);
            for (e, p) in f.effect.iter_mut().zip([px, py, px, py]) {
                // Zero adds no effects room when rotation contracts this axis.
                if *e != 0.0 {
                    *e = (*e - p).clamp(0.0, 1584.0);
                }
            }
        }
        Some(obj)
    }

    /// Keep the selected native-ink branch and its picture fallback as one object. Reading only
    /// the fallback flattens the ink; reading only the choice loses its preview on the next save.
    pub(super) fn read_ink_alternate(&mut self, sc: &mut StoryCtx, ac: &El, rels: &Rels) -> Option<InlineObject> {
        let choice = children_of_choice(ac).filter(|e| e.name == "mc:Choice")?;
        let drawing = choice.child("w:drawing")?;
        if drawing.find("a:graphicData")?.attr("uri") != Some(INK_URI) {
            return None;
        }
        let fallback = ac.child("mc:Fallback");
        let preview = fallback.and_then(|f| f.child("w:pict")).and_then(|p| self.read_vml(sc, p, rels));
        let Some(mut object) = self.read_drawing(sc, drawing, rels) else { return preview };
        if let InlineObject::Graphic { w, h, graphic, .. } = &mut object {
            let graphic = Arc::make_mut(graphic);
            if let Some(InlineObject::Image { media, .. }) = preview {
                graphic.items = vec![GraphicItem::Image { rect: [0.0, 0.0, *w, *h], media }];
            }
            if let Some(fallback) = fallback {
                let kept =
                    El { name: "mc:AlternateContent".into(), attrs: Vec::new(), kids: vec![Node::El(choice.clone()), Node::El(fallback.clone())] };
                if let Some(source) = self.keep_embedded(&kept, rels, &[]) {
                    graphic.source = Some(source);
                }
                // A malformed or over-budget fallback must not discard a valid native part.
            }
        }
        Some(object)
    }

    /// An Office 2016 chart (`cx:chart`: waterfall, sunburst, box & whisker…) drawn from its
    /// fallback picture, keeping both branches so saving writes the chart back rather than only
    /// the picture. `None` (read the fallback alone) when it can't be kept whole.
    pub(super) fn read_chartex_alternate(&mut self, sc: &mut StoryCtx, ac: &El, rels: &Rels) -> Option<InlineObject> {
        let choice = ac
            .children("mc:Choice")
            .find(|c| c.child("w:drawing").and_then(|d| d.find("a:graphicData")).and_then(|g| g.attr("uri")) == Some(CHARTEX_URI))?;
        // A requirement in a namespace we can't name couldn't be written back.
        let requires = choice.attr("Requires").unwrap_or("");
        if requires.split_whitespace().next().is_none() || requires.split_whitespace().any(|p| p == "?") {
            return None;
        }
        let fallback = ac.child("mc:Fallback")?;
        let kept = El { name: "mc:AlternateContent".into(), attrs: Vec::new(), kids: vec![Node::El(choice.clone()), Node::El(fallback.clone())] };
        let source = self.keep_embedded(&kept, rels, &[])?;
        let Some(InlineObject::Image { media, w, h, alt, float, .. }) = self.read_drawing(sc, fallback.child("w:drawing")?, rels) else {
            return None;
        };
        let items = vec![GraphicItem::Image { rect: [0.0, 0.0, w, h], media }];
        let graphic = Arc::new(Graphic { kind: GraphicKind::Chart, items, w, h, source: Some(source), ..Default::default() });
        Some(InlineObject::Graphic { w, h, alt, float, graphic })
    }

    /// A group or canvas member that is native ink (`content`, a `w14:contentPart`, inside
    /// `alternate` when it has a picture fallback): what to write back (the whole alternate when
    /// it can be kept, else the ink alone) and its fallback picture (a `Document::media` key).
    /// `None` when the ink's part is missing or isn't InkML.
    fn ink_member(&mut self, content: &El, alternate: Option<&El>, rels: &Rels) -> Option<(Arc<Embedded>, Option<String>)> {
        let InlineObject::Graphic { graphic, .. } = self.read_ink_content_part(content, rels)? else { return None };
        let mut source = graphic.source.clone()?;
        let mut preview = None;
        let choice = alternate.and_then(children_of_choice).filter(|c| c.name == "mc:Choice");
        if let (Some(choice), Some(fallback)) = (choice, alternate.and_then(|a| a.child("mc:Fallback"))) {
            let kept = El { name: "mc:AlternateContent".into(), attrs: Vec::new(), kids: vec![Node::El(choice.clone()), Node::El(fallback.clone())] };
            // A malformed or over-budget fallback must not discard a valid native part.
            if let Some(whole) = self.keep_embedded(&kept, rels, &[]) {
                source = whole;
                preview = match fallback.child("pic:pic").and_then(|p| self.read_pic(p, rels, 0.0, 0.0, String::new(), Float::default())) {
                    Some(InlineObject::Image { media, .. }) => Some(media),
                    _ => None,
                };
            }
        }
        Some((source, preview))
    }

    pub(super) fn read_ink_content_part(&mut self, content: &El, rels: &Rels) -> Option<InlineObject> {
        let part = crate::read::part_of(rels, content.attr("r:id")?, INK_REL)?;
        let pkg = self.pkg;
        if self.content_types.get_or_insert_with(|| ContentTypes::read(pkg)).of(&part) != Some("application/inkml+xml") {
            return None;
        }
        let source = self.keep_embedded(content, rels, &[])?;
        let graphic = Arc::new(Graphic { kind: GraphicKind::Ink, source: Some(source), ..Default::default() });
        Some(InlineObject::Graphic { w: 0.0, h: 0.0, alt: String::new(), float: Float::default(), graphic })
    }

    /// The chart or SmartArt diagram in an `a:graphicData`, as items in points from its top-left
    /// corner. Charts are drawn from their cached data, SmartArt diagrams from the drawing Word
    /// stores beside them. Built once per part and size; once the file's graphic budget is spent,
    /// later ones are empty.
    fn graphic(&mut self, kind: GraphicKind, gd: &El, rels: &Rels, w: f32, h: f32, source: Option<Arc<Embedded>>) -> Arc<Graphic> {
        let part = match kind {
            GraphicKind::Chart => gd.child("c:chart").and_then(|c| c.attr("r:id")).and_then(|id| crate::read::part_of(rels, id, rt::CHART)),
            GraphicKind::Diagram => self.diagram_part(gd, rels),
            GraphicKind::Ink => None,
        };
        let Some(path) = part else { return Arc::new(Graphic { kind, items: Vec::new(), w, h, source, chart: None, smart_art: None }) };
        let key = (kind, path, w.to_bits(), h.to_bits());
        if let Some(g) = self.graphics.get(&key) {
            return g.clone();
        }
        let items = if self.graphic_budget == 0 { Vec::new() } else { self.graphic_items(kind, &key.1, w, h) };
        self.graphic_budget = self.graphic_budget.saturating_sub(graphic_work(&items));
        // A chart or SmartArt that is exactly what WordCraft writes for its model can be edited
        // (and is written from the model); any other stays as it was read.
        let chart = match kind {
            GraphicKind::Chart if !items.is_empty() => self.graphic_part(&key.1).and_then(|space| crate::chart_spec::spec_of(&space)).map(Arc::new),
            _ => None,
        };
        let smart_art = match kind {
            GraphicKind::Diagram if !items.is_empty() => self.smart_art_spec(gd, rels).map(Arc::new),
            _ => None,
        };
        let source = if chart.is_some() || smart_art.is_some() { None } else { source };
        let g = Arc::new(Graphic { kind, items, w, h, source, chart, smart_art });
        self.graphics.insert(key, g.clone());
        g
    }

    /// The items of the chart part or diagram drawing part at `path`.
    fn graphic_items(&mut self, kind: GraphicKind, path: &str, w: f32, h: f32) -> Vec<GraphicItem> {
        match kind {
            GraphicKind::Chart => match self.graphic_part(path) {
                Some(space) => crate::read::chart::chart_items(&space, &self.doc.settings.theme_colors, w, h),
                None => Vec::new(),
            },
            GraphicKind::Diagram => self.diagram_drawing(path, w, h),
            GraphicKind::Ink => Vec::new(),
        }
    }

    /// A picture: the first `a:blip` in `pic` and its crop.
    fn read_pic(&mut self, pic: &El, rels: &Rels, w: f32, h: f32, alt: String, mut float: Float) -> Option<InlineObject> {
        let blip = pic.find("a:blip")?;
        float.set_spin(xfrm_spin(pic.find("pic:spPr")));
        let media = blip.attr("r:embed").and_then(|id| self.media_for(rels, id))?;
        let mut crop = [0.0f32; 4];
        if let Some(sr) = pic.find("a:srcRect") {
            for (i, n) in ["l", "t", "r", "b"].iter().enumerate() {
                if let Some(v) = sr.attr(n).and_then(int)
                    && let Some(slot) = crop.get_mut(i)
                {
                    *slot = (v.clamp(0, 100_000) as f32 / 100_000.0).clamp(0.0, 1.0);
                }
            }
        }
        Some(InlineObject::Image { media, w, h, alt, float, crop, ole: None })
    }

    /// A `wpg:wgp` group (`w` × `h`): its pictures, shapes and text boxes, nested groups
    /// flattened into it. `None` when it has no member we can show.
    fn read_group(&mut self, sc: &mut StoryCtx, g: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> Option<InlineObject> {
        let x = group_xfrm(g.child("wpg:grpSpPr"));
        float.set_spin(xfrm_spin(g.child("wpg:grpSpPr")));
        // The members' space; without `a:chExt` it is the group's own size.
        let (ch_w, ch_h) = match x.ch_ext {
            Some((cw, ch)) if cw > 0.0 && ch > 0.0 => (cw, ch),
            _ => (w.max(1.0), h.max(1.0)),
        };
        let (cx, cy) = x.ch_off.unwrap_or((0.0, 0.0));
        let mut children = Vec::new();
        let outer = self.group_fill.take();
        self.group_fill = own_fill(g.child("wpg:grpSpPr"), &self.doc.settings.theme_colors, None).flatten();
        self.group_members(sc, g, rels, (-cx, -cy, 1.0, 1.0), 0, &mut children);
        self.group_fill = outer;
        if children.is_empty() {
            return None;
        }
        Some(InlineObject::Group { w, h, float, ch_w, ch_h, children, canvas: None })
    }

    /// A `wpc:wpc` Drawing Canvas (`w` × `h`, ECMA-376 Part 1 §20.4 and the Word 2010 canvas
    /// schema): its `wpc:bg` fill, `wpc:whole` outline and members, which sit in the canvas's
    /// own space (offsets from its top-left corner, unscaled). An empty canvas is kept.
    /// The canvas schema has no `a:xfrm` of its own (nor does the anchor), so a turned or
    /// flipped canvas keeps its turn in our `wpc:extLst` extension ([`CANVAS_SPIN_EXT`]).
    fn read_canvas(&mut self, sc: &mut StoryCtx, c: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> InlineObject {
        let spin_ext = c.child("wpc:extLst").and_then(|l| l.els().find(|e| e.name == "a:ext" && e.attr("uri") == Some(crate::CANVAS_SPIN_EXT)));
        float.set_spin(xfrm_spin(spin_ext));
        let solid = |e: Option<&El>| {
            e.and_then(|f| f.child("a:solidFill")).and_then(|f| f.child("a:srgbClr")).and_then(|c| c.attr("val")).and_then(Rgb::parse)
        };
        let ln = c.child("wpc:whole").and_then(|e| e.child("a:ln"));
        let style = wordcraft_doc::para::CanvasStyle {
            fill: own_fill(c.child("wpc:bg"), &self.doc.settings.theme_colors, None).flatten(),
            stroke: solid(ln),
            stroke_width: ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 100.0),
        };
        let mut children = Vec::new();
        self.group_members(sc, c, rels, (0.0, 0.0, 1.0, 1.0), 0, &mut children);
        InlineObject::Group { w, h, float, ch_w: w.max(1.0), ch_h: h.max(1.0), children, canvas: Some(style) }
    }

    /// The members of group `g` into `out`, mapped into the outermost group's space by `t`
    /// (x0, y0, sx, sy: outer = x0 + inner × sx).
    fn group_members(
        &mut self,
        sc: &mut StoryCtx,
        g: &El,
        rels: &Rels,
        t: (f32, f32, f32, f32),
        depth: usize,
        out: &mut Vec<wordcraft_doc::para::GroupChild>,
    ) {
        let (x0, y0, sx, sy) = t;
        for e in g.els() {
            if out.len() >= wordcraft_doc::para::MAX_GROUP_CHILDREN {
                return;
            }
            // Native ink (`w14:contentPart`), alone or with a picture fallback: kept whole, in its
            // place among the members. Ink that can't be kept gives way to its fallback.
            let (e, ink) = match e.name.as_str() {
                "w14:contentPart" => (e, self.ink_member(e, None, rels)),
                "mc:AlternateContent" => {
                    let choice = children_of_choice(e);
                    match choice.filter(|c| c.name == "mc:Choice").and_then(|c| c.els().next()).filter(|c| c.name == "w14:contentPart") {
                        Some(content) => match self.ink_member(content, Some(e), rels) {
                            Some(ink) => (content, Some(ink)),
                            None => match e.child("mc:Fallback").and_then(|f| f.els().next()) {
                                Some(c) => (c, None),
                                None => continue,
                            },
                        },
                        None => match choice.and_then(|c| c.els().next()) {
                            Some(c) => (c, None),
                            None => continue,
                        },
                    }
                }
                _ => (e, None),
            };
            let sppr = match e.name.as_str() {
                "wps:wsp" => e.child("wps:spPr"),
                "pic:pic" => e.child("pic:spPr"),
                "w14:contentPart" if ink.is_some() => Some(e),
                // A group inside a canvas is a whole `wpg:wgp`.
                "wpg:grpSp" | "wpg:wgp" => e.child("wpg:grpSpPr"),
                _ => continue,
            };
            let x = group_xfrm(sppr);
            let ((ox, oy), (ew, eh)) = (x.off, x.ext);
            if e.name == "wpg:grpSp" || e.name == "wpg:wgp" {
                if depth >= 8 {
                    continue;
                }
                // Its members' space maps onto its own box.
                let (cox, coy) = x.ch_off.unwrap_or((0.0, 0.0));
                let (cw, ch) = x.ch_ext.filter(|(cw, ch)| *cw > 0.0 && *ch > 0.0).unwrap_or((ew.max(1.0), eh.max(1.0)));
                let (kx, ky) = (ew / cw, eh / ch);
                let nt = (x0 + (ox - cox * kx) * sx, y0 + (oy - coy * ky) * sy, sx * kx, sy * ky);
                // Its own fill, for its members' `a:grpFill` (#1500).
                // One without a fill of its own passes its group's on.
                let own = own_fill(sppr, &self.doc.settings.theme_colors, self.group_fill.as_ref()).unwrap_or_else(|| self.group_fill.clone());
                let outer = std::mem::replace(&mut self.group_fill, own);
                self.group_members(sc, e, rels, nt, depth + 1, out);
                self.group_fill = outer;
                continue;
            }
            let max = crate::units::MAX_LEN_PT;
            let fit = |v: f32, lo: f32| wordcraft_geom::finite(v).clamp(lo, max);
            let (w, h) = (fit(ew * sx, 0.0), fit(eh * sy, 0.0));
            let nv = match e.name.as_str() {
                "pic:pic" => e.find("pic:cNvPr"),
                "w14:contentPart" => e.child("w14:nvContentPartPr").and_then(|p| p.child("w14:cNvPr")),
                _ => e.child("wps:cNvPr"),
            };
            let (alt, meta) = nv_alt(nv);
            let float = Float { alt: meta, hidden: nv.is_some_and(|p| on_off_attr(p, "hidden")), ..Float::default() };
            let obj = if let Some((source, preview)) = ink {
                let mut float = float;
                float.set_spin(xfrm_spin(e.child("w14:xfrm")));
                // As drawing-level ink: its fallback picture, else the missing-picture box.
                let items = vec![GraphicItem::Image { rect: [0.0, 0.0, w, h], media: preview.unwrap_or_default() }];
                let graphic = Arc::new(Graphic { kind: GraphicKind::Ink, items, w, h, source: Some(source), ..Default::default() });
                Some(InlineObject::Graphic { w, h, alt: alt.clone(), float, graphic })
            } else if e.name == "pic:pic" {
                self.read_pic(e, rels, w, h, alt.clone(), float)
            } else {
                let mut o = self.read_wsp(sc, e, rels, w, h, float);
                self.set_shape_id(&mut o, e.child("wps:cNvPr").and_then(|p| p.attr("id")));
                Some(o)
            };
            let obj = obj.map(|mut o| {
                o.set_alt_text(&alt);
                o
            });
            if let Some(obj) = obj {
                out.push(wordcraft_doc::para::GroupChild { x: fit(x0 + ox * sx, -max), y: fit(y0 + oy * sy, -max), obj });
            }
        }
    }

    /// Build a shape from `wps:wsp`, using frame dimensions `w` and `h` in points and the caller's
    /// placement. Reads geometry, paint, effects and connector ends, and creates a text-box story
    /// when present within the nesting limit. Explicit outline caps are preserved; missing or
    /// unknown caps use flat ends, without resolving theme-inherited caps.
    fn read_wsp(&mut self, sc: &mut StoryCtx, wsp: &El, rels: &Rels, w: f32, h: f32, mut float: Float) -> InlineObject {
        let sppr = wsp.child("wps:spPr");
        float.set_spin(xfrm_spin(sppr));
        let prst = sppr.and_then(|s| s.child("a:prstGeom")).and_then(|g| g.attr("prst")).unwrap_or("rect");
        let txbx = wsp.child("wps:txbx").and_then(|t| t.child("w:txbxContent"));
        let mut kind = preset_kind(prst);
        let is_tb = wsp.child("wps:cNvSpPr").and_then(|c| c.attr("txBox")).is_some_and(|v| v == "1" || v == "true");
        if is_tb || (txbx.is_some() && kind == ShapeKind::Rectangle) {
            kind = ShapeKind::TextBox;
        }
        // Explicit paint, else the theme's through the style references (#611).
        let paint = shape_paint(sppr, wsp.child("wps:style"), &self.doc.settings.theme_colors, &self.theme_fills, self.group_fill.as_ref());
        let (stroke, stroke_width) = (paint.stroke, paint.stroke_width);
        let fill = if kind.is_open() { None } else { paint.fill };
        let ln = sppr.and_then(|s| s.child("a:ln"));
        let effects = sppr.and_then(|s| s.child("a:effectLst")).map(effect_list).unwrap_or_default();
        let story = match txbx {
            Some(t) if sc.story_depth < MAX_STORY_DEPTH => Some(self.read_textbox(sc, t, rels)),
            _ => None,
        };
        // A box continuing another's text (Create Link): its own, empty story, linked up once the
        // whole document is read.
        let linked = wsp.child("wps:linkedTxbx").filter(|_| txbx.is_none());
        let story = match linked {
            Some(l) => {
                kind = ShapeKind::TextBox;
                let id = l.attr("id").and_then(|v| v.trim().parse::<u32>().ok());
                let seq = l.attr("seq").and_then(|v| v.trim().parse::<u32>().ok());
                match (id, seq) {
                    (Some(id), Some(seq)) if seq > 0 => {
                        let part = self.doc.add_part(PartKind::TextBox, Blocks::new());
                        self.txbx_links.push((id, seq, part));
                        Some(part)
                    }
                    _ => story,
                }
            }
            None => {
                if let (Some(part), Some(id)) = (story, wsp.child("wps:txbx").and_then(|t| t.attr("id")).and_then(|v| v.trim().parse::<u32>().ok())) {
                    self.txbx_links.push((id, 0, part));
                }
                story
            }
        };
        // Text direction and alignment (`wps:bodyPr`).
        if let (Some(id), Some(bp)) = (story, wsp.child("wps:bodyPr"))
            && let Some(part) = self.doc.parts.get_mut(&id)
        {
            if let Some(v) = bp.attr("vert") {
                part.body.vert = wordcraft_doc::props::TextVert::from_ooxml(v);
            }
            if let Some(a) = bp.attr("anchor") {
                part.body.anchor = wordcraft_doc::props::valign_from_anchor(a);
            }
            // Internal margins, each defaulting to Word's (#743).
            let mut inset = wordcraft_doc::TEXT_BOX_INSET;
            for (slot, n) in inset.iter_mut().zip(["lIns", "tIns", "rIns", "bIns"]) {
                if let Some(v) = bp.attr(n).and_then(int) {
                    *slot = u32::try_from(v.clamp(0, i64::from(wordcraft_doc::MAX_TEXT_BOX_INSET))).unwrap_or(0);
                }
            }
            part.body.inset = (inset != wordcraft_doc::TEXT_BOX_INSET).then_some(inset);
            part.body.auto_fit = bp.child("a:spAutoFit").is_some();
        }
        // The shape style's text colour (`a:fontRef`, often `lt1`: white on the accent fill).
        if let Some(id) = story
            && let Some(c) = wsp.child("wps:style").and_then(|s| s.child("a:fontRef"))
            && let Some(rgb) = crate::read::drawing_color::color(c, &self.doc.settings.theme_colors)
            && let Some(part) = self.doc.parts.get_mut(&id)
        {
            part.body.auto_color = Some(rgb);
        }
        let mut extra = ShapeExtra {
            dash: crate::read::shape_style::line_dash(ln),
            cap: wordcraft_doc::connector::LineCap::from_ooxml(ln.and_then(|e| e.attr("cap")).unwrap_or("flat")),
            outline_fill: paint.outline_fill,
            ..ShapeExtra::default()
        };
        // A rounded rectangle's own corner (`adj`, ECMA-376 §20.1.10.56); other presets drawn as
        // rounded rectangles keep the default.
        if prst == "roundRect" {
            extra.corner = sppr
                .and_then(|s| s.child("a:prstGeom"))
                .and_then(|g| preset_adj(g, "adj"))
                .and_then(|v| u32::try_from(v.clamp(0, i64::from(wordcraft_doc::connector::MAX_CORNER))).ok());
        }
        // Arrowheads: any end type but `none` (ECMA-376 §20.1.10.33) is drawn as a triangle.
        let arrow = |tag: &str| ln.and_then(|l| l.child(tag)).and_then(|e| e.attr("type")).is_some_and(|t| t != "none");
        extra.arrow_start = arrow("a:headEnd");
        extra.arrow_end = arrow("a:tailEnd");
        // A connector's glued ends (ECMA-376 §20.1.2.2.36, §20.1.2.2.13).
        if let Some(cn) = wsp.child("wps:cNvCnPr") {
            let end = |tag: &str| {
                let e = cn.child(tag)?;
                let id = e.attr("id").and_then(u32_of).filter(|v| *v != 0)?;
                let site = e.attr("idx").and_then(u32_of).unwrap_or(0).min(wordcraft_doc::connector::MAX_SITE);
                Some(ConnEnd { id, site })
            };
            extra.start = end("a:stCxn");
            extra.end = end("a:endCxn");
            self.glued.extend(extra.start.iter().chain(extra.end.iter()).map(|e| e.id));
        }
        extra.warp = wsp.child("wps:bodyPr").and_then(|b| b.child("a:prstTxWarp")).and_then(text_warp);
        // A freeform: its custom geometry (#743: in a group too).
        let freeform = sppr.and_then(|s| crate::read::freeform::cust_geom(s, w, h)).map(|mut f| {
            f.alpha = crate::read::freeform::line_alpha(ln);
            kind = ShapeKind::Freeform;
            Arc::new(f)
        });
        InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, freeform, effects, extra }
    }

    /// Keep drawing id `id` on a shape read from a file (see [`Reader::glued`]).
    fn set_shape_id(&mut self, obj: &mut InlineObject, id: Option<&str>) {
        if let InlineObject::Shape { extra, .. } = obj
            && let Some(id) = id.and_then(u32_of).filter(|v| *v != 0)
        {
            extra.id = id;
            self.shape_ids = true;
        }
    }

    pub(super) fn read_textbox(&mut self, sc: &StoryCtx, content: &El, rels: &Rels) -> u32 {
        let mut inner = StoryCtx { story_depth: sc.story_depth + 1, ..Default::default() };
        let mut blocks = Blocks::new();
        // Shapes in the text aren't members of a group the box may be in.
        let group = self.group_fill.take();
        self.read_blocks(&mut inner, content, rels, &mut blocks, 0);
        self.group_fill = group;
        self.flush_pending(&mut inner, &mut blocks);
        self.doc.add_part(PartKind::TextBox, blocks)
    }
}

/// The alternative text of a `wp:docPr`, `pic:cNvPr` or `wps:cNvPr` (ECMA-376
/// `CT_NonVisualDrawingProps`): the description (the title when there is none, as before), and
/// the title and decorative mark (the `adec:decorative` extension of [MS-ODRAWXML]).
fn nv_alt(p: Option<&El>) -> (String, AltText) {
    let Some(p) = p else { return Default::default() };
    let title = cap_alt(p.attr("title").unwrap_or(""));
    let descr = p.attr("descr").filter(|s| !s.is_empty()).map(cap_alt).unwrap_or_else(|| title.clone());
    let decorative = p
        .child("a:extLst")
        .into_iter()
        .flat_map(|l| l.children("a:ext"))
        .filter(|x| x.attr("uri").is_some_and(|u| u.eq_ignore_ascii_case(crate::xml::DECORATIVE_EXT)))
        .flat_map(|x| x.els())
        .any(|d| d.local() == "decorative" && on_off_attr(d, "val"));
    (descr, AltText { text: String::new(), title, decorative })
}

fn anchor_float(c: &El) -> Float {
    // `relativeHeight`: the stacking order Word draws floating objects in (higher on top), which
    // need not be their order in the file.
    let z = c.attr("relativeHeight").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
    let mut f = Float { wrap: Wrap::Square, z, ..Float::default() };
    let behind = on_off_attr(c, "behindDoc");
    // Word wraps a position relative to the page's size (`wp14:pctPosVOffset`) in
    // `mc:AlternateContent`; its Fallback gives the same position as an offset (#743).
    let kids = c.els().flat_map(|k| match k.name.as_str() {
        "mc:AlternateContent" => k.child("mc:Fallback").map(|f| f.els().collect()).unwrap_or_default(),
        _ => vec![k],
    });
    for k in kids {
        match k.name.as_str() {
            "wp:wrapSquare" => {
                f.wrap = Wrap::Square;
                f.wrap_text = k.attr("wrapText").and_then(WrapText::from_ooxml).unwrap_or_default();
            }
            "wp:wrapTight" | "wp:wrapThrough" => {
                f.wrap = if k.name == "wp:wrapTight" { Wrap::Tight } else { Wrap::Through };
                f.wrap_text = k.attr("wrapText").and_then(WrapText::from_ooxml).unwrap_or_default();
                f.wrap_polygon = k.child("wp:wrapPolygon").and_then(wrap_polygon).map(Arc::new);
            }
            "wp:wrapTopAndBottom" => f.wrap = Wrap::TopAndBottom,
            "wp:wrapNone" => f.wrap = if behind { Wrap::BehindText } else { Wrap::InFrontOfText },
            "wp:positionH" | "wp:positionV" => {
                let horiz = k.name == "wp:positionH";
                let rel = match (k.attr("relativeFrom").unwrap_or(""), horiz) {
                    ("page", _) => Anchor::Page,
                    ("margin", _) => Anchor::Margin,
                    ("leftMargin", true) => Anchor::LeftMargin,
                    ("rightMargin", true) => Anchor::RightMargin,
                    ("insideMargin", _) => Anchor::InsideMargin,
                    ("outsideMargin", _) => Anchor::OutsideMargin,
                    ("character", true) => Anchor::Character,
                    ("topMargin", false) => Anchor::TopMargin,
                    ("bottomMargin", false) => Anchor::BottomMargin,
                    ("line", false) => Anchor::Line,
                    ("paragraph", false) => Anchor::Paragraph,
                    (_, true) => Anchor::Column,
                    (_, false) => Anchor::Paragraph,
                };
                let align = k.child("wp:align").and_then(|a| FloatAlign::from_ooxml(a.text().trim()));
                let off = k.child("wp:posOffset").and_then(|o| measure(&o.text(), 12_700.0)).unwrap_or(0.0);
                if horiz {
                    f.h_rel = rel;
                    f.h_align = align;
                    f.x = off;
                } else {
                    f.v_rel = rel;
                    f.v_align = align;
                    f.y = off;
                }
            }
            _ => {}
        }
    }
    let dist = |n: &str| c.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0).clamp(0.0, 1584.0);
    f.dist = dist("distL").max(dist("distR"));
    f.dist_top = dist("distT");
    f.dist_bottom = dist("distB");
    // Required by the schema; a file leaving it out lays out in the cell, as before.
    f.outside_cell = !on_off_attr(c, "layoutInCell") && c.attr("layoutInCell").is_some();
    f
}

/// A `wp:wrapPolygon`: `wp:start`, then the `wp:lineTo` points (ECMA-376 Part 1, §20.4.2.16).
fn wrap_polygon(e: &El) -> Option<wordcraft_doc::wrap::WrapPolygon> {
    let pts = e
        .els()
        .filter(|k| k.name == "wp:start" || k.name == "wp:lineTo")
        .take(wordcraft_doc::wrap::MAX_WRAP_POINTS + 1)
        .filter_map(|k| Some((int(k.attr("x")?)?, int(k.attr("y")?)?)));
    let mut pts: Vec<(i64, i64)> = pts.collect();
    // The last point usually repeats the start to close the polygon.
    if pts.len() > 3 && pts.first() == pts.last() {
        pts.pop();
    }
    let edited = on_off_attr(e, "edited");
    // An unedited polygon round the whole frame says nothing the object doesn't (it's what we
    // write when there's no outline): keep deriving the outline.
    let s = i64::from(wordcraft_doc::wrap::WRAP_SPACE);
    let mut corners = pts.clone();
    corners.sort_unstable();
    if !edited && corners == [(0, 0), (0, s), (s, 0), (s, s)] {
        return None;
    }
    wordcraft_doc::wrap::WrapPolygon::new(edited, pts)
}

/// What a graphic's items cost to keep and draw: one per item, plus one per path segment.
fn graphic_work(items: &[GraphicItem]) -> usize {
    items.iter().map(|it| if let GraphicItem::Path { segs, .. } = it { segs.len() + 1 } else { 1 }).sum()
}

/// A shape's `a:effectLst` (ECMA-376 Part 1 §20.1.8.26): its outer shadow, glow and soft edges.
/// Other effects are dropped.
/// A preset text warp (`a:prstTxWarp`, ECMA-376 §20.1.9.19) and its adjust values
/// (`a:avLst/a:gd` with `fmla="val N"`). Unknown presets are dropped.
fn text_warp(e: &El) -> Option<TextWarp> {
    let mut w = TextWarp::new(e.attr("prst")?)?;
    for gd in e.child("a:avLst").into_iter().flat_map(|l| l.children("a:gd")) {
        let (Some(name), Some(f)) = (gd.attr("name"), gd.attr("fmla")) else { continue };
        if let Some(v) = f.trim().strip_prefix("val").and_then(|v| v.trim().parse::<i64>().ok()) {
            w.set_adj(name, v);
        }
    }
    Some(w)
}

/// A preset geometry's adjust value `name` (`a:avLst/a:gd` with `fmla="val N"`).
fn preset_adj(geom: &El, name: &str) -> Option<i64> {
    let gd = geom.child("a:avLst")?.children("a:gd").find(|g| g.attr("name") == Some(name))?;
    gd.attr("fmla")?.trim().strip_prefix("val")?.trim().parse::<i64>().ok()
}

fn effect_list(l: &El) -> ShapeEffects {
    let pt = |e: &El, n: &str| e.attr(n).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.0);
    let shadow = l.child("a:outerShdw").map(|e| {
        let (color, transparency) = effect_color(e);
        // `dir`: 60000ths of a degree, clockwise.
        let angle = e.attr("dir").and_then(int).map(|v| (v.rem_euclid(21_600_000) as f32) / 60_000.0).unwrap_or(0.0);
        // `rotWithShape` is true unless it says otherwise (ECMA-376 §20.1.8.49).
        let rot_with_shape = e.attr("rotWithShape").is_none_or(|v| !matches!(v, "0" | "false" | "off"));
        Shadow { color: color.unwrap_or(Rgb::BLACK), transparency, blur: pt(e, "blurRad"), distance: pt(e, "dist"), angle, rot_with_shape }
    });
    let glow = l.child("a:glow").map(|e| {
        let (color, transparency) = effect_color(e);
        Glow { color: color.unwrap_or(Glow::default().color), size: pt(e, "rad"), transparency }
    });
    let soft_edge = l.child("a:softEdge").map(|e| pt(e, "rad"));
    ShapeEffects { shadow, glow, soft_edge }.sanitized()
}

/// The colour inside an effect (`a:srgbClr`, `a:prstClr`, `a:sysClr`; theme colours aren't
/// resolved here) and its transparency, percent (from `a:alpha`, opaque when absent).
fn effect_color(e: &El) -> (Option<Rgb>, f32) {
    let Some(c) = e.els().find(|c| matches!(c.name.as_str(), "a:srgbClr" | "a:prstClr" | "a:sysClr" | "a:schemeClr" | "a:scrgbClr" | "a:hslClr"))
    else {
        return (None, 0.0);
    };
    let rgb = match c.name.as_str() {
        "a:srgbClr" => c.attr("val").and_then(Rgb::parse),
        "a:sysClr" => c.attr("lastClr").and_then(Rgb::parse),
        "a:prstClr" => match c.attr("val").unwrap_or("") {
            "black" => Some(Rgb::BLACK),
            "white" => Some(Rgb::WHITE),
            "gray" | "grey" => Some(Rgb(0x80, 0x80, 0x80)),
            _ => None,
        },
        _ => None,
    };
    // `a:alpha`: opacity in 1000ths of a percent.
    let opacity = c.child("a:alpha").and_then(|a| a.attr("val")).and_then(int).map(|v| v.clamp(0, 100_000) as f32 / 1000.0).unwrap_or(100.0);
    (rgb, 100.0 - opacity)
}

/// A DrawingML `a:xfrm` (ECMA-376 §20.1.7.5/6) in points: offset, extent and, for a group, its
/// members' offset and extent.
struct Xfrm {
    off: (f32, f32),
    ext: (f32, f32),
    ch_off: Option<(f32, f32)>,
    ch_ext: Option<(f32, f32)>,
}

/// The rotation and flips of the `a:xfrm` in shape properties `sppr` (ECMA-376 §20.1.7.6:
/// `rot` in 60000ths of a degree, clockwise; `flipH`, `flipV`). None when missing or junk.
fn xfrm_spin(sppr: Option<&El>) -> wordcraft_geom::Spin {
    let Some(x) = sppr.and_then(|s| if matches!(s.name.as_str(), "a:xfrm" | "w14:xfrm") { Some(s) } else { s.child("a:xfrm") }) else {
        return wordcraft_geom::Spin::default();
    };
    let rot = x.attr("rot").and_then(int).map(|v| v.rem_euclid(21_600_000) as f32 / 60_000.0).unwrap_or(0.0);
    wordcraft_geom::Spin::new(rot, on_off_attr(x, "flipH"), on_off_attr(x, "flipV"))
}

/// The `a:xfrm` in shape properties `sppr` (zeros when missing).
fn group_xfrm(sppr: Option<&El>) -> Xfrm {
    // A content part (native ink) has its own `w14:xfrm`.
    let x = sppr.and_then(|s| s.child("a:xfrm").or_else(|| s.child("w14:xfrm")));
    let pair = |name: &str, a: &str, b: &str, lim: f32| {
        let e = x.and_then(|x| x.child(name))?;
        let v = |n: &str| e.attr(n).and_then(|v| measure(v, 12_700.0)).map(|v| v.clamp(-lim, lim)).unwrap_or(0.0);
        Some((v(a), v(b)))
    };
    let max = crate::units::MAX_LEN_PT;
    let pos = |p: Option<(f32, f32)>| p.map(|(a, b)| (a.max(0.0), b.max(0.0)));
    Xfrm {
        off: pair("a:off", "x", "y", max).unwrap_or((0.0, 0.0)),
        ext: pos(pair("a:ext", "cx", "cy", max)).unwrap_or((0.0, 0.0)),
        ch_off: pair("a:chOff", "x", "y", max),
        ch_ext: pos(pair("a:chExt", "cx", "cy", max)),
    }
}
