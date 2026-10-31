use super::*;

fn owned_geometry(p: &ParaLayout) -> usize {
    p.glyphs.capacity() * std::mem::size_of::<para::Glyph>()
        + p.clusters.capacity() * std::mem::size_of::<para::Cluster>()
        + p.lines.capacity() * std::mem::size_of::<para::Line>()
        + p.lines.iter().map(|l| l.xs.capacity() * 4 + l.vis.capacity() * std::mem::size_of::<para::VisCl>()).sum::<usize>()
}

#[test]
fn obsolete_long_paragraph_geometry_has_a_byte_budget() {
    let mut doc = Document::from_text(&"long paragraph words ".repeat(1300));
    let mut cache = LayoutCache::new();
    let mut versions = Vec::new();
    for _ in 0..40 {
        doc.insert_text(&Pos::body(0, 0), "x ", &Default::default()).unwrap();
        let laid = layout(&doc, &mut cache, &LayoutOptions::default());
        let para = laid
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .find_map(|p| match p {
                Placed::Lines { para, .. } => Some(para),
                _ => None,
            })
            .unwrap();
        versions.push((Arc::downgrade(para), owned_geometry(para)));
    }
    let retained: usize = versions.iter().filter(|(p, _)| p.strong_count() > 0).map(|(_, bytes)| bytes).sum();
    assert!(retained <= 64 * 1024 * 1024, "obsolete paragraph geometry retained {retained} bytes");
    assert!(versions.last().unwrap().0.strong_count() > 0, "current layout stays cached");
    let misses = cache.misses;
    layout(&doc, &mut cache, &LayoutOptions::default());
    assert_eq!(cache.misses, misses, "unchanged frame still hits the cache");
}

#[test]
fn byte_pressure_never_evicts_current_multi_paragraph_layouts() {
    let doc = Document::from_text(&["first paragraph ".repeat(100), "second paragraph ".repeat(100)].join("\n"));
    let mut cache = LayoutCache::new();
    layout(&doc, &mut cache, &LayoutOptions::default());
    let current: Vec<_> = cache.paras.values().map(Arc::downgrade).collect();
    assert_eq!(current.len(), 2);
    cache.prune_unused(1);
    assert!(current.iter().all(|p| p.strong_count() > 0));
    let misses = cache.misses;
    layout(&doc, &mut cache, &LayoutOptions::default());
    assert_eq!(cache.misses, misses);
}

#[test]
fn byte_pressure_preserves_page_total_reflow_and_current_geometry() {
    let mut doc = Document::from_text("first\nsecond\nthird");
    for i in 0..3 {
        let para = doc.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        para.props.page_break_before = Some(i > 0);
        para.insert_object(
            para.len(),
            InlineObject::Field { instr: "NUMPAGES".into(), result: "9".into(), locked: false, dirty: false, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
    }
    let mut cache = LayoutCache::new();
    layout(&doc, &mut cache, &LayoutOptions::default());
    doc.insert_text(&Pos::body(0, 0), "changed ", &Default::default()).unwrap();
    let warm = layout(&doc, &mut cache, &LayoutOptions::default());
    cache.prune_unused(1);
    let after = layout(&doc, &mut cache, &LayoutOptions::default());
    let cold = lay(&doc);
    for laid in [&warm, &after, &cold] {
        assert_eq!(laid.pages.len(), 3);
        for page in &laid.pages {
            let values: Vec<_> = page
                .items
                .iter()
                .flat_map(|p| match p {
                    Placed::Lines { para, .. } => para.shown.iter().map(|s| s.1.as_str()).collect(),
                    _ => Vec::new(),
                })
                .collect();
            assert_eq!(values, ["3"]);
        }
        for i in 0..3 {
            assert_eq!(format!("{:?}", laid.caret(&Pos::body(i, 0))), format!("{:?}", cold.caret(&Pos::body(i, 0))));
        }
    }
}
