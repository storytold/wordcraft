//! Format crates registered with the I/O dispatcher.

use wordcraft_doc::Document;

/// Open a format other than plain text / JSON; files that give no page size (Markdown, HTML)
/// start on `page`.
pub fn open(ext: &str, bytes: &[u8], page: &wordcraft_doc::SectionProps) -> Option<Result<Document, String>> {
    match ext {
        "docx" | "docm" | "dotx" | "dotm" => Some(crate::io::read_docx(bytes, None).map_err(|e| e.to_string())),
        "doc" | "dot" => Some(wordcraft_docbin::read(bytes).map_err(|e| e.to_string())),
        // Word XML Document (Flat OPC) or Word 2003 XML, told apart by the root element.
        "xml" => Some(wordcraft_docx::read_xml(bytes).map_err(|e| e.to_string())),
        // HTML, Markdown, RTF…: pictures fit the text width (see `io::fit_pictures`). An
        // OpenDocument file says how big they are, and one wider than the text is meant to be;
        // changing it would take its paragraph off being written as read. Its reader fits those
        // that state no size.
        other => wordcraft_formats::import_on(other, bytes, page).map(|r| {
            r.map(|mut doc| {
                let odf = matches!(other, "odt" | "ott" | "fodt");
                if !odf {
                    crate::io::fit_pictures(&mut doc);
                }
                draw_charts(&mut doc, odf);
                doc
            })
        }),
    }
}

/// Draw the charts a reader left undrawn (ODT charts come as their chart model alone): in the
/// document's theme colours, at their size. With `odf`, scatter series are set to markers without
/// a line first (see `markers_only_scatter`).
pub(crate) fn draw_charts(doc: &mut Document, odf: bool) {
    use wordcraft_doc::para::InlineObject;
    /// Groups nest at most this deep here (the readers flatten deeper ones).
    const DEPTH: usize = 16;
    fn undrawn(o: &InlineObject, depth: usize) -> bool {
        match o {
            InlineObject::Graphic { graphic, .. } => graphic.chart.is_some() && graphic.items.is_empty(),
            InlineObject::Group { children, .. } if depth < DEPTH => children.iter().any(|c| undrawn(&c.obj, depth + 1)),
            _ => false,
        }
    }
    fn draw(o: &mut InlineObject, theme: &[wordcraft_doc::Rgb], odf: bool, depth: usize) {
        match o {
            InlineObject::Group { children, .. } if depth < DEPTH => children.iter_mut().for_each(|c| draw(&mut c.obj, theme, odf, depth + 1)),
            o if undrawn(o, depth) => {
                if odf {
                    markers_only_scatter(o);
                }
                crate::cmd::chart::redraw(o, theme)
            }
            _ => {}
        }
    }
    // The ODF reader does not read a scatter chart's line setting, so every scatter series of an
    // opened ODF file is set to markers without a line, as the ODF writer writes them. A series
    // with its markers turned off gets them back, as Change Chart Type does, so it is still drawn.
    fn markers_only_scatter(o: &mut InlineObject) {
        if let InlineObject::Graphic { graphic, .. } = o
            && let Some(spec) = graphic.chart.as_deref()
            && spec.kind == wordcraft_doc::chart::ChartType::Scatter
            && spec.series.iter().any(|s| !s.no_line || s.markers == Some(false))
        {
            let mut spec = spec.clone();
            for series in &mut spec.series {
                series.no_line = true;
                if series.markers == Some(false) {
                    series.markers = None;
                }
            }
            std::sync::Arc::make_mut(graphic).chart = Some(std::sync::Arc::new(spec));
        }
    }
    let theme = doc.settings.theme_colors.clone();
    // (Markup kept as read for these charts is still written as read after they are drawn,
    // scatter charts included.)
    doc.fill_objects(&|o| undrawn(o, 0), &mut |o| draw(o, &theme, odf, 0));
}

/// Save to a format other than plain text / JSON.
pub fn save(ext: &str, doc: &Document, opts: &crate::io::ExportOptions) -> Option<Result<Vec<u8>, String>> {
    // .docx/.docm/.dotx/.dotm: the extension picks the package flavour Word expects.
    if let Some(flavor) = wordcraft_docx::Flavor::from_ext(ext) {
        return Some(wordcraft_docx::write_as(doc, flavor).map_err(|e| e.to_string()));
    }
    match ext {
        "xml" if opts.word2003 => Some(wordcraft_docx::write_wordml2003(doc).map_err(|e| e.to_string())),
        "xml" => Some(wordcraft_docx::write_flat(doc).map_err(|e| e.to_string())),
        "png" => Some(render_png(doc, 0, 2.0, opts)),
        "pdf" => {
            // Output shows the fields Word fills in as it prints (see `output_fields`).
            let out = crate::cmd::references::output_fields(doc);
            let doc = out.as_ref().unwrap_or(doc);
            Some(crate::strict::export_pdf(doc, opts))
        }
        other => wordcraft_formats::export_with(other, doc, &wordcraft_formats::ExportOptions { comments: opts.markup && opts.comments }),
    }
}

/// How a page image is drawn: the printed page, headers and footers at full strength, tracked
/// changes marked as Track Changes Options mark them. One place for `file.exportPng`,
/// `file.strictReport`'s page check and a downloaded page image.
pub(crate) fn page_image_options(opts: &crate::io::ExportOptions) -> wordcraft_render::RenderOptions {
    let mut ro = wordcraft_render::RenderOptions { display: crate::strict::png_display(), ..Default::default() };
    ro.display.revisions = opts.revisions.clone();
    ro
}

/// Render page `page` as PNG at `scale` px/pt. With Print Markup's comments the page has its
/// markup area with the comments in balloons (page images always show tracked changes).
pub fn render_png(doc: &Document, page: usize, scale: f32, opts: &crate::io::ExportOptions) -> Result<Vec<u8>, String> {
    let out = crate::cmd::references::output_fields(doc);
    let doc = out.as_ref().unwrap_or(doc);
    let lo = wordcraft_layout::LayoutOptions {
        comment_balloons: opts.markup && opts.comments,
        hide_marked: opts.revisions.hidden_deletions(),
        numeral: opts.numeral,
        output_page_refs: true,
        bookmark_error: crate::locale::bookmark_error(),
        ..Default::default()
    };
    let l = wordcraft_layout::layout(doc, &mut wordcraft_layout::LayoutCache::new(), &lo);
    let p = l.pages.get(page).ok_or_else(|| format!("no page {}", page + 1))?;
    if opts.strict {
        let display = page_image_options(opts).display;
        let report = crate::strict::strict_report(doc, &l, &display, Some(page..=page), wordcraft_layout::StrictOutput::Png);
        if !report.passed() {
            return Err(report.refusal());
        }
    }
    let img = wordcraft_render::render_page(doc, p, scale, &page_image_options(opts));
    Ok(img.to_png())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page image is the printed page: headers and footers draw at full strength (#6267).
    #[test]
    fn a_page_image_draws_headers_at_full_strength() {
        let opts = crate::io::ExportOptions::default();
        let ro = page_image_options(&opts);
        assert!(!ro.display.dim_header, "a page image is the printed page");
        assert_eq!(ro.display.revisions, opts.revisions, "the Track Changes Options reach the image");

        let mut d = Document::from_text("body");
        let png = wordcraft_geom::picture::encode_format(
            &image::DynamicImage::from(image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 0, 0, 255]))),
            image::ImageFormat::Png,
        )
        .unwrap();
        let media = d.add_media(png, "png");
        let mut hp = wordcraft_doc::Paragraph::new();
        let obj = wordcraft_doc::para::InlineObject::Image {
            media,
            w: 40.0,
            h: 40.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        hp.insert_object(0, obj, &Default::default()).unwrap();
        let id = d.add_part(wordcraft_doc::PartKind::Header, vec![wordcraft_doc::para_block(hp)]);
        d.last_section.headers.default = Some(id);

        let rendered = render_png(&d, 0, 1.0, &opts).unwrap();
        let l = wordcraft_layout::layout(&d, &mut wordcraft_layout::LayoutCache::new(), &wordcraft_layout::LayoutOptions::default());
        let full = wordcraft_render::render_page(&d, &l.pages[0], 1.0, &ro).to_png();
        let dimmed = wordcraft_render::render_page(&d, &l.pages[0], 1.0, &wordcraft_render::RenderOptions::default()).to_png();
        assert_eq!(rendered, full, "render_png draws headers at full strength");
        assert_ne!(rendered, dimmed, "render_png does not dim headers");
    }
}
