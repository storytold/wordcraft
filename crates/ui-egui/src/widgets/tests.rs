/// A row of a combo's list: a dim section heading nothing picks, or an entry.
enum Row<'a> {
    Heading(&'a str),
    Item(&'a String),
}

/// Rows in a list of `n` entries headed by `recent` recent ones: the two headings and the recent
/// entries come on top when there are any.
fn section_rows(recent: usize, n: usize) -> usize {
    if recent == 0 { n } else { n.saturating_add(recent).saturating_add(2) }
}

/// Row `k` of a list headed by the `recent` families: Recently Used Fonts, those families, All
/// Fonts, then `item(0..)`. Without recent families the rows are the items.
fn section_row<'a>(k: usize, recent: &'a [String], item: &dyn Fn(usize) -> Option<&'a String>) -> Option<Row<'a>> {
    if recent.is_empty() {
        return item(k).map(Row::Item);
    }
    if k == 0 {
        return Some(Row::Heading("Recently Used Fonts"));
    }
    if let Some(r) = recent.get(k.saturating_sub(1)) {
        return Some(Row::Item(r));
    }
    if k == recent.len().saturating_add(1) {
        return Some(Row::Heading("All Fonts"));
    }
    item(k.saturating_sub(recent.len()).saturating_sub(2)).map(Row::Item)
}

/// The row of entry `entry` in a list headed by `recent` recent families ([`section_row`]).
fn highlight_row(entry: usize, recent: usize) -> usize {
    if recent == 0 {
        entry
    } else if entry < recent {
        entry.saturating_add(1)
    } else {
        entry.saturating_add(2)
    }
}

#[test]
fn font_list_sections_skip_their_headings() {
    let items: Vec<String> = ["Arial", "Courier New"].map(String::from).to_vec();
    let recent: Vec<String> = ["Georgia"].map(String::from).to_vec();
    fn names<'a>(items: &'a [String], recent: &'a [String], n: usize) -> Vec<String> {
        let item = |k: usize| items.get(k);
        (0..n)
            .map(|k| match section_row(k, recent, &item) {
                Some(Row::Heading(h)) => format!("[{h}]"),
                Some(Row::Item(i)) => i.clone(),
                None => "-".into(),
            })
            .collect()
    }
    assert_eq!(section_rows(1, 2), 5);
    assert_eq!(names(&items, &recent, 6), ["[Recently Used Fonts]", "Georgia", "[All Fonts]", "Arial", "Courier New", "-"]);
    assert_eq!([highlight_row(0, 1), highlight_row(1, 1), highlight_row(2, 1)], [1, 3, 4]);
    // Nothing recent: no headings, the rows are the items.
    assert_eq!(section_rows(0, 2), 2);
    assert_eq!(names(&items, &[], 3), ["Arial", "Courier New", "-"]);
    assert_eq!(highlight_row(1, 0), 1);
}
