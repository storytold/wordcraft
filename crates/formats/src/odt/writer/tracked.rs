//! Tracked changes written (ODF 1.3 §5.5): changed regions, insertions open around the text, and
//! deleted content, paragraph marks included.

use super::{FBlock, HashMap, Inline, Para, RevMark, Writer, x};

/// One piece of a paragraph being written: an inline, or the deleted mark of a paragraph
/// joined to the next one.
pub(in crate::odt) enum Item<'a> {
    Inl(&'a Inline),
    Mark(&'a Para),
}

impl Item<'_> {
    /// Its tracked insertion and deletion.
    pub(in crate::odt) fn marks(&self) -> (Option<&RevMark>, Option<&RevMark>) {
        match self {
            Item::Inl(Inline::Text(_, f) | Inline::Ruby { fmt: f, .. } | Inline::Field { fmt: f, .. }) => (f.ins.as_ref(), f.del.as_ref()),
            Item::Inl(_) => (None, None),
            Item::Mark(p) => (p.mark_ins.as_deref(), p.mark_del.as_deref()),
        }
    }

    /// What a run of content is written as: deleted content is one region whatever was
    /// inserted inside it, so only its deletion counts.
    pub(in crate::odt) fn segment(&self) -> (Option<&RevMark>, Option<&RevMark>) {
        match self.marks() {
            (_, Some(del)) => (None, Some(del)),
            k => k,
        }
    }

    pub(in crate::odt) fn inline(&self) -> Option<&Inline> {
        match self {
            Item::Inl(i) => Some(i),
            Item::Mark(_) => None,
        }
    }
}

/// Close the insertion open in `st`.
pub(in crate::odt) fn ins_close(st: &mut Option<(RevMark, String)>, out: &mut String) {
    if let Some((_, ct)) = st.take() {
        out.push_str(&format!("<text:change-end text:change-id=\"{ct}\"/>"));
    }
}

/// The blocks after joining at deleted marks, and each joined run's paragraphs by the first one's index.
pub(in crate::odt) type JoinedMarks = (Vec<FBlock>, HashMap<usize, Vec<Para>>);

/// `blocks` with each run of paragraphs joined at tracked deleted paragraph marks (ODF keeps
/// such a deletion inside one body paragraph): the first paragraph stands for the run, and the
/// whole run is returned by its index. `None` when nothing is joined. A deleted mark with no
/// paragraph after it (before a table, at the end) is written as a plain mark.
pub(in crate::odt) fn join_deleted_marks(blocks: &[FBlock]) -> Option<JoinedMarks> {
    if !blocks.iter().any(|b| matches!(b, FBlock::Para(p) if p.mark_del.is_some())) {
        return None;
    }
    let mut out = Vec::with_capacity(blocks.len());
    let mut chains = HashMap::new();
    let mut i = 0;
    while let Some(b) = blocks.get(i) {
        i += 1;
        let FBlock::Para(p) = b else {
            out.push(b.clone());
            continue;
        };
        let mut chain = vec![p.clone()];
        while chain.last().is_some_and(|q| q.mark_del.is_some())
            && let Some(FBlock::Para(q)) = blocks.get(i)
        {
            chain.push(q.clone());
            i += 1;
        }
        if chain.len() > 1 {
            chains.insert(out.len(), chain);
        }
        out.push(FBlock::Para(p.clone()));
    }
    Some((out, chains))
}

/// Now, as an `xsd:dateTime` (UTC).
pub(in crate::odt) fn now_iso() -> String {
    let t = now_unix();
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Reuse the crate's 400-year-cycle converter (chrono within a cycle, cycles carried in the
    // year) so a clock past chrono's range keeps the proleptic Gregorian date the old hand-rolled
    // code gave, never the epoch. `{y:04}` rather than `%Y`, which writes a `+` before year 10000.
    let (y, m, d) = crate::sheet::civil(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Seconds since the Unix epoch: the system clock, or the browser's on the web.
pub(in crate::odt) fn now_unix() -> i64 {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as i64 } else { 0 }
    }
}

impl Writer {
    /// A new `text:changed-region` of `kind` by `m` with `body` after its change info; its id.
    pub(in crate::odt) fn region(&mut self, kind: &str, m: &RevMark, body: &str) -> String {
        // (With the writer's prefix: a file's own regions, written as read, keep their ids.)
        let id = format!("{}ct{}", self.prefix, self.regions.len() + 1);
        // `dc:date` is required: a change without a date is dated now.
        let date = if m.date.trim().is_empty() { now_iso() } else { m.date.clone() };
        self.regions.push(format!(
            "<text:changed-region xml:id=\"{id}\" text:id=\"{id}\"><text:{kind}><office:change-info><dc:creator>{}</dc:creator><dc:date>{}</dc:date></office:change-info>{body}</text:{kind}></text:changed-region>",
            x(&m.author),
            x(&date)
        ));
        id
    }

    /// Make `m` the insertion open in `st` (closing another one first).
    pub(in crate::odt) fn ins_open(&mut self, st: &mut Option<(RevMark, String)>, m: &RevMark, out: &mut String) {
        if st.as_ref().is_some_and(|(o, _)| o == m) {
            return;
        }
        ins_close(st, out);
        let ct = self.region("insertion", m, "");
        out.push_str(&format!("<text:change-start text:change-id=\"{ct}\"/>"));
        *st = Some((m.clone(), ct));
    }

    pub(in crate::odt) fn open_ins(&mut self, m: &RevMark, out: &mut String) {
        let mut st = self.open_ins.take();
        self.ins_open(&mut st, m, out);
        self.open_ins = st;
    }

    pub(in crate::odt) fn close_ins(&mut self, out: &mut String) {
        ins_close(&mut self.open_ins, out);
    }

    /// Inlines with their insertions as change ranges kept in `st`.
    pub(in crate::odt) fn ins_inlines(&mut self, items: &[Item<'_>], st: &mut Option<(RevMark, String)>, out: &mut String) {
        let mut k = 0;
        while k < items.len() {
            let ins = items.get(k).and_then(|i| i.marks().0);
            let mut j = k + 1;
            while j < items.len() && items.get(j).and_then(|i| i.marks().0) == ins {
                j += 1;
            }
            match ins {
                Some(m) => self.ins_open(st, m, out),
                None => ins_close(st, out),
            }
            let seg: Vec<Inline> = items.get(k..j).unwrap_or(&[]).iter().filter_map(|i| i.inline().cloned()).collect();
            self.plain_inlines(&seg, out);
            k = j;
        }
    }

    /// Deleted content: a region holding it (one paragraph per deleted paragraph mark plus
    /// one; text inserted before it was deleted as change ranges inside) and a `text:change`
    /// point in the body.
    pub(in crate::odt) fn deletion(&mut self, seg: &[Item<'_>], m: &RevMark, out: &mut String) {
        let mut body = String::new();
        let mut st: Option<(RevMark, String)> = None;
        let mut start = 0;
        let mut first = true;
        loop {
            let end = seg.iter().skip(start).position(|i| matches!(i, Item::Mark(_))).map(|e| start + e);
            if !first {
                self.part += 1;
            }
            first = false;
            let style = self.chain.get(self.part).cloned().unwrap_or_else(|| self.cur_style.clone());
            body.push_str(&format!("<text:p text:style-name=\"{style}\">"));
            self.ins_inlines(seg.get(start..end.unwrap_or(seg.len())).unwrap_or(&[]), &mut st, &mut body);
            // A deleted paragraph mark that had been inserted: the range runs on past it.
            match end.and_then(|e| seg.get(e)).and_then(|i| i.marks().0) {
                Some(ins) => self.ins_open(&mut st, ins, &mut body),
                None => ins_close(&mut st, &mut body),
            }
            body.push_str("</text:p>");
            match end {
                Some(e) => start = e + 1,
                None => break,
            }
        }
        let ct = self.region("deletion", m, &body);
        out.push_str(&format!("<text:change text:change-id=\"{ct}\"/>"));
    }

    /// Paragraph content, with tracked insertions as change ranges and deletions as change points.
    pub(in crate::odt) fn items(&mut self, items: &[Item<'_>], out: &mut String) {
        let mut k = 0;
        while k < items.len() {
            let key = items.get(k).map(Item::segment).unwrap_or_default();
            let mut j = k + 1;
            while j < items.len() && items.get(j).map(Item::segment).unwrap_or_default() == key {
                j += 1;
            }
            let seg = items.get(k..j).unwrap_or(&[]);
            match key {
                (_, Some(del)) => self.deletion(seg, del, out),
                (Some(ins), None) => {
                    self.open_ins(ins, out);
                    let inl: Vec<Inline> = seg.iter().filter_map(|i| i.inline().cloned()).collect();
                    self.plain_inlines(&inl, out);
                }
                (None, None) => {
                    self.close_ins(out);
                    let inl: Vec<Inline> = seg.iter().filter_map(|i| i.inline().cloned()).collect();
                    self.plain_inlines(&inl, out);
                }
            }
            k = j;
        }
    }
}
