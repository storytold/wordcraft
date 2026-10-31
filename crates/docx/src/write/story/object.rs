use super::drawing::{DrawingInfo, shape_name, spin_attrs, write_fill, xfrm_spin_only};
use super::field::{FieldData, field_run};
use super::section::comment_reference_style;
use super::{CharProps, Fill, InlineObject, NoteKind, PartRels, TextColor, W, Wrap, Writer, emu, n, rpr};

impl Writer<'_> {
    pub(super) fn object(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        match o {
            // Written by `para_content` (inline) or `blocks` (around paragraphs).
            InlineObject::ControlStart { .. } | InlineObject::ControlEnd => {}
            InlineObject::BookmarkStart { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkStart", &[("w:id", &id), ("w:name", name)]);
            }
            InlineObject::BookmarkEnd { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkEnd", &[("w:id", &id)]);
            }
            InlineObject::PermStart { id, group, editor } => {
                let id = id.to_string();
                let mut a = vec![("w:id", id.as_str())];
                if !group.is_empty() {
                    a.push(("w:edGrp", group.as_str()));
                }
                if !editor.is_empty() {
                    a.push(("w:ed", editor.as_str()));
                }
                w.empty("w:permStart", &a);
            }
            InlineObject::PermEnd { id } => w.empty("w:permEnd", &[("w:id", &id.to_string())]),
            InlineObject::CommentStart { id } => {
                if self.doc.comments.contains_key(id) {
                    w.empty("w:commentRangeStart", &[("w:id", &id.to_string())]);
                }
            }
            InlineObject::CommentEnd { id } => {
                if self.doc.comments.contains_key(id) {
                    let s = id.to_string();
                    w.empty("w:commentRangeEnd", &[("w:id", &s)]);
                    w.open("w:r", &[]);
                    w.open("w:rPr", &[]);
                    w.val("w:rStyle", comment_reference_style(self.doc));
                    w.close("w:rPr");
                    w.empty("w:commentReference", &[("w:id", &s)]);
                    w.close("w:r");
                }
            }
            InlineObject::Equation { linear, display, math } => {
                // Automatic equation numbers become text: Word numbers nothing by itself.
                let color = match props.color {
                    Some(TextColor::Rgb(c)) => Some(c),
                    _ => None,
                };
                match crate::write::math::resolve_numbers(math, *display, &mut self.eq_number) {
                    Some(m) => crate::write::math::write_equation(w, linear, *display, &m, color),
                    None => crate::write::math::write_equation(w, linear, *display, math, color),
                }
            }
            InlineObject::Field { instr, result, locked, dirty, form, .. } => {
                self.field(w, FieldData { instr, result, locked: *locked, dirty: *dirty, form }, props)
            }
            InlineObject::FieldStart { instr, locked, dirty } => {
                self.rev_open(w, props);
                self.field_start(w, instr, *locked, *dirty, "", props);
                self.rev_close(w, props);
            }
            InlineObject::FieldEnd => {
                self.rev_open(w, props);
                field_run(w, props, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
                self.rev_close(w, props);
            }
            InlineObject::NoteRef { kind, id, custom } => {
                let foot = *kind == NoteKind::Footnote;
                if self.current_note == Some((foot, *id)) {
                    // The note's own number is the w:footnoteRef / w:endnoteRef mark, written once
                    // per note: here for its first reference to itself, never as a reference.
                    if let Some(mark) = self.pending_mark.take() {
                        let mut p = props.clone();
                        if p.style.is_none() && p.vert_align.is_none() {
                            p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                        }
                        self.rev_open(w, props);
                        w.open("w:r", &[]);
                        self.rpr(w, &p);
                        // A custom mark is plain text; the placeholder would turn it into a number.
                        if custom.is_empty() {
                            w.empty(mark, &[]);
                        } else {
                            w.leaf("w:t", &[("xml:space", "preserve")], custom);
                        }
                        w.close("w:r");
                        self.rev_close(w, props);
                    }
                    return;
                }
                let nid = self.note_id(*id, foot);
                let mut p = props.clone();
                if p.style.is_none() && p.vert_align.is_none() {
                    p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                }
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, &p);
                let tag = if foot { "w:footnoteReference" } else { "w:endnoteReference" };
                if custom.is_empty() {
                    w.empty(tag, &[("w:id", &nid)]);
                } else {
                    w.empty(tag, &[("w:customMarkFollows", "1"), ("w:id", &nid)]);
                    w.leaf("w:t", &[("xml:space", "preserve")], custom);
                }
                w.close("w:r");
                self.rev_close(w, props);
            }
            // Editable charts and SmartArt keep main's model writer; opaque graphics and native
            // ink keep their original source and related parts.
            InlineObject::Graphic { .. } => self.write_graphic(w, o, props, rels, depth),
            InlineObject::Image { .. } => self.write_image(w, o, props, rels, depth),
            InlineObject::Shape { .. } => self.write_shape(w, o, props, rels, depth),
            InlineObject::Group { .. } => self.write_group(w, o, props, rels, depth),
            InlineObject::Ruby { base, ruby, align, size, raise, base_size, lang, props: ruby_props } => {
                // ECMA-376 §17.3.3.25: a run holding w:ruby (properties, ruby text, base text).
                let t = if self.is_del(props) { "w:delText" } else { "w:t" };
                let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
                let hps = |v: f32, min: f32| n(wordcraft_geom::to_half_points(v.clamp(min, 1584.0)));
                let size = fin(*size, 5.0).clamp(1.0, 1584.0);
                let base_size = if base_size.is_finite() && *base_size > 0.0 { *base_size } else { props.size.unwrap_or(10.5) };
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, props);
                w.open("w:ruby", &[]);
                w.open("w:rubyPr", &[]);
                w.val("w:rubyAlign", align.ooxml());
                w.val("w:hps", &hps(size, 1.0));
                w.val("w:hpsRaise", &hps(fin(*raise, 0.0), 0.0));
                w.val("w:hpsBaseText", &hps(fin(base_size, 10.5), 1.0));
                w.val("w:lid", if lang.is_empty() { "ja-JP" } else { lang });
                w.close("w:rubyPr");
                let mut rt_props = props.clone().overlaid(ruby_props);
                rt_props.size = Some(size);
                for (tag, p, text) in [("w:rt", &rt_props, ruby), ("w:rubyBase", props, base)] {
                    w.open(tag, &[]);
                    w.open("w:r", &[]);
                    rpr(w, p);
                    w.leaf(t, &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    w.close(tag);
                }
                w.close("w:ruby");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::PositionalTab { align, relative_to, leader } => {
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                w.empty(
                    "w:ptab",
                    &[("w:relativeTo", relative_to.ooxml()), ("w:alignment", align.ooxml()), ("w:leader", crate::write::props::tab_leader(*leader))],
                );
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Opaque { format, xml, text } => {
                if format == "docx" && crate::xml::parse(xml.as_bytes()).is_ok() {
                    w.raw(xml);
                } else if !text.is_empty() {
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    self.rpr(w, props);
                    w.leaf("w:t", &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    self.rev_close(w, props);
                }
            }
        }
    }

    fn write_graphic(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, _depth: usize) {
        let InlineObject::Graphic { w: gw, h: gh, alt, float, graphic } = o else { return };
        let ink = graphic.kind == wordcraft_doc::graphic::GraphicKind::Ink;
        let root = match (graphic.chart.as_deref(), graphic.smart_art.as_deref(), graphic.source.as_deref()) {
            (Some(spec), _, _) => crate::xml::parse(self.chart_graphic(spec, rels).as_bytes()).ok(),
            (None, Some(spec), _) => crate::xml::parse(self.smart_art_graphic(spec, *gw, *gh, rels).as_bytes()).ok(),
            // Ink in a group member's form (no drawing of its own) can't stand alone: left
            // out before its parts are queued, so no orphan part or relationship is written.
            (None, None, Some(src)) if ink && !standalone_ink(src) => {
                log::warn!("docx: group-member ink outside its group can't be written; it is left out");
                None
            }
            (None, None, Some(src)) => self.embedded_tree(src, rels, ink.then_some((*gw, *gh, float, alt.as_str()))),
            (None, None, None) => None,
        };
        let Some(mut root) = root else { return };
        let direct_ink = ink && root.name == "w:contentPart";
        if ink && !direct_ink {
            Self::ink_transform(&mut root, (graphic.w, graphic.h), (*gw, *gh), float, 0);
        }
        // Native ink, or an Office 2016 chart (`cx:chart`) with its fallback picture.
        let alternate = (root.name == "mc:AlternateContent").then_some(&root);
        let fallback = alternate.and_then(|e| e.child("mc:Fallback"));
        let choice = alternate.and_then(|e| e.child("mc:Choice"));
        let native = choice.and_then(|e| e.find("a:graphic"));
        if alternate.is_some() && native.is_none() {
            return;
        }
        // A chart's picture gets the chart's current frame too; ink's VML preview is resized in place.
        let picture = if ink { None } else { fallback.and_then(|e| e.find("a:graphic")) };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        if direct_ink {
            w.raw(&root.to_xml());
        } else {
            if fallback.is_some() {
                let requires = if ink { "wpi" } else { choice.and_then(|c| c.attr("Requires")).unwrap_or_default() };
                w.open("mc:AlternateContent", &[]);
                w.open("mc:Choice", &[("Requires", requires)]);
            }
            let docpr = self.next_docpr();
            let name = match graphic.kind {
                wordcraft_doc::graphic::GraphicKind::Chart => format!("Chart {docpr}"),
                wordcraft_doc::graphic::GraphicKind::Diagram => format!("Diagram {docpr}"),
                wordcraft_doc::graphic::GraphicKind::Ink => format!("Ink {docpr}"),
            };
            let z = self.z;
            self.graphic_frame(w, DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt }, native.unwrap_or(&root));
            if let Some(fallback) = fallback {
                w.close("mc:Choice");
                match picture {
                    Some(picture) => {
                        // Both branches are one object: same id and stacking order.
                        self.z = z;
                        w.open("mc:Fallback", &[]);
                        self.graphic_frame(w, DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt }, picture);
                        w.close("mc:Fallback");
                    }
                    None => w.raw(&fallback.to_xml()),
                }
                w.close("mc:AlternateContent");
            }
        }
        w.close("w:r");
        self.rev_close(w, props);
    }

    fn write_image(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, _depth: usize) {
        let InlineObject::Image { media, w: iw, h: ih, alt, float, ole, .. } = o else { return };
        // An OLE object read from a file: the object itself, at its current size.
        if let Some(src) = ole.as_deref()
            && let Some(obj) = self.embedded_xml(src, rels, Some((*iw, *ih, float, alt.as_str())))
        {
            self.rev_open(w, props);
            w.open("w:r", &[]);
            self.rpr(w, props);
            w.raw(&obj);
            w.close("w:r");
            self.rev_close(w, props);
            return;
        }
        let Some(file) = self.media_files.get(media).cloned() else { return };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:drawing", &[]);
        let docpr = self.next_docpr();
        let name = format!("Picture {docpr}");
        self.drawing_open(w, DrawingInfo { float, size: (*iw, *ih), docpr: &docpr, name: &name, alt });
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        w.open("a:graphicData", &[("uri", "http://schemas.openxmlformats.org/drawingml/2006/picture")]);
        self.pic(w, o, &file, (0.0, 0.0), rels);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        w.close("w:r");
        self.rev_close(w, props);
    }

    fn write_shape(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        let InlineObject::Shape { kind, w: sw, h: sh, float, effects, freeform, extra, .. } = o else { return };
        // An ActiveX control read from a file: the control itself, at its current size.
        if let Some(src) = extra.ole.as_deref()
            && let Some(obj) = self.embedded_xml(src, rels, Some((*sw, *sh, float, o.alt_text())))
        {
            self.rev_open(w, props);
            w.open("w:r", &[]);
            self.rpr(w, props);
            w.raw(&obj);
            w.close("w:r");
            self.rev_close(w, props);
            return;
        }
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:drawing", &[]);
        let docpr = self.shape_docpr(extra.id);
        // Ink is told apart by its name (and its pen), which reading looks for.
        let name = shape_name(*kind, freeform.as_deref(), &docpr);
        // The effect extent leaves room for the shadow and glow.
        let mut float = float.clone();
        for (e, fx) in float.effect.iter_mut().zip(effects.extent()) {
            *e = wordcraft_geom::finite(*e).max(fx);
        }
        // Tight and Through wrapping need a polygon: the shape's outline unless it has one.
        if matches!(float.wrap, Wrap::Tight | Wrap::Through) && float.wrap_polygon.is_none() {
            float.wrap_polygon = wordcraft_doc::wrap::shape_outline(*kind, *sw, *sh, extra.corner)
                .and_then(|pts| wordcraft_doc::wrap::WrapPolygon::from_unit(false, &pts))
                .map(std::sync::Arc::new);
        }
        let float = &float;
        self.drawing_open(w, DrawingInfo { float, size: (*sw, *sh), docpr: &docpr, name: &name, alt: o.alt_text() });
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape")]);
        self.wsp(w, o, None, (0.0, 0.0), rels, depth);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        w.close("w:r");
        self.rev_close(w, props);
    }

    fn write_group(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        let InlineObject::Group { w: gw, h: gh, float, ch_w, ch_h, children, canvas } = o else { return };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:drawing", &[]);
        let docpr = self.next_docpr();
        let name = format!("{} {docpr}", if canvas.is_some() { "Canvas" } else { "Group" });
        self.drawing_open(w, DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt: o.alt_text() });
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        let outer = match canvas {
            Some(c) => {
                // A Drawing Canvas: its background (`wpc:bg`) and outline (`wpc:whole`), then
                // its members, at offsets in the canvas's own (unscaled) space.
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas")]);
                w.open("wpc:wpc", &[]);
                w.open("wpc:bg", &[]);
                write_fill(w, c.fill.as_ref());
                w.close("wpc:bg");
                w.open("wpc:whole", &[]);
                match c.stroke {
                    Some(_) => w.open("a:ln", &[("w", &emu(c.stroke_width.clamp(0.0, 100.0)))]),
                    None => w.open("a:ln", &[]),
                }
                write_fill(w, c.stroke.map(Fill::solid).as_ref());
                w.close("a:ln");
                w.close("wpc:whole");
                "wpc:wpc"
            }
            None => {
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup")]);
                // ECMA-376 §20.1.7.6 (`a:xfrm` of a group): the members' space `a:chOff`/`a:chExt`
                // maps onto the group's `a:off`/`a:ext`.
                w.open("wpg:wgp", &[]);
                w.empty("wpg:cNvGrpSpPr", &[]);
                w.open("wpg:grpSpPr", &[]);
                w.open("a:xfrm", &spin_attrs(float.spin()).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
                w.empty("a:off", &[("x", "0"), ("y", "0")]);
                w.empty("a:ext", &[("cx", &emu(gw.max(0.0))), ("cy", &emu(gh.max(0.0)))]);
                w.empty("a:chOff", &[("x", "0"), ("y", "0")]);
                w.empty("a:chExt", &[("cx", &emu(ch_w.max(0.0))), ("cy", &emu(ch_h.max(0.0)))]);
                w.close("a:xfrm");
                w.close("wpg:grpSpPr");
                "wpg:wgp"
            }
        };
        for c in children.iter().take(wordcraft_doc::para::MAX_GROUP_CHILDREN) {
            let at = (c.x, c.y);
            match &c.obj {
                InlineObject::Image { media, .. } => {
                    if let Some(file) = self.media_files.get(media).cloned() {
                        self.pic(w, &c.obj, &file, at, rels);
                    }
                }
                InlineObject::Shape { extra, .. } => {
                    let id = self.shape_docpr(extra.id);
                    self.wsp(w, &c.obj, Some(&id), at, rels, depth);
                }
                // Native ink kept from the file, with its fallback, at the member's place.
                InlineObject::Graphic { w: iw, h: ih, float, graphic, .. } if graphic.kind == wordcraft_doc::graphic::GraphicKind::Ink => {
                    if let Some(src) = graphic.source.as_deref()
                        && let Some(mut root) = self.embedded_tree(src, rels, None)
                    {
                        Self::place_member(&mut root, [c.x, c.y, *iw, *ih], float, 0);
                        w.raw(&root.to_xml());
                    }
                }
                _ => {}
            }
        }
        if canvas.is_some() && float.spin() != wordcraft_geom::Spin::default() {
            // The canvas schema has no transform of its own: keep its turn in our
            // extension (Word draws it unturned, as it can't turn a canvas).
            w.open("wpc:extLst", &[]);
            w.open("a:ext", &[("uri", crate::CANVAS_SPIN_EXT)]);
            xfrm_spin_only(w, float.spin());
            w.close("a:ext");
            w.close("wpc:extLst");
        }
        w.close(outer);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        w.close("w:r");
        self.rev_close(w, props);
    }
}

pub(super) fn ink_name(tool: wordcraft_doc::freeform::InkTool) -> &'static str {
    match tool {
        wordcraft_doc::freeform::InkTool::Pen => "Pen",
        wordcraft_doc::freeform::InkTool::Pencil => "Pencil",
        wordcraft_doc::freeform::InkTool::Highlighter => "Highlighter",
    }
}

/// Is kept ink `src` in a form the drawing-level writer writes: its `a:graphic`, a direct run-level
/// `w:contentPart`, or an `mc:AlternateContent` whose choice holds a drawing (not a group member's
/// bare `w14:contentPart`, nor one in `mc:AlternateContent` with a `pic:pic` fallback)?
fn standalone_ink(src: &wordcraft_doc::graphic::Embedded) -> bool {
    match crate::xml::parse(src.xml.as_bytes()) {
        Ok(root) => match root.name.as_str() {
            "a:graphic" | "w:contentPart" => true,
            "mc:AlternateContent" => root.child("mc:Choice").and_then(|c| c.child("w:drawing")).is_some_and(|d| d.find("a:graphic").is_some()),
            _ => false,
        },
        Err(_) => false,
    }
}

#[cfg(test)]
mod standalone_ink_tests {
    use super::standalone_ink;
    use wordcraft_doc::graphic::Embedded;

    fn emb(xml: &str) -> Embedded {
        Embedded { xml: xml.into(), ..Default::default() }
    }

    /// Review: an alternate whose choice has a drawing but no `a:graphic` is not writable as ink.
    #[test]
    fn alternate_content_needs_a_graphic() {
        let ns = r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        let with =
            format!("<mc:AlternateContent {ns}><mc:Choice Requires=\"w14\"><w:drawing><a:graphic/></w:drawing></mc:Choice></mc:AlternateContent>");
        let without = format!("<mc:AlternateContent {ns}><mc:Choice Requires=\"w14\"><w:drawing/></mc:Choice></mc:AlternateContent>");
        assert!(standalone_ink(&emb(&with)));
        assert!(!standalone_ink(&emb(&without)));
    }
}
