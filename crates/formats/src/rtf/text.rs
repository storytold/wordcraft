use super::state::{Dest, MAX_PICT, St};
use super::{Fmt, Inline, Reader, Rgb, make_img, model};

/// The encoding of an East Asian multibyte code page (932 Shift-JIS, 936 GBK, 949 UHC, 950 Big5),
/// which [`Reader::finish_multibyte`] decodes a run with; `None` for the single-byte code pages.
fn multibyte_encoding(code_page: u16) -> Option<&'static encoding_rs::Encoding> {
    let label = wordcraft_doc::encoding::multibyte_encoding_name(code_page)?;
    encoding_rs::Encoding::for_label(label.as_bytes())
}

impl Reader {
    pub(super) fn commit_font(&mut self) {
        let name = std::mem::take(&mut self.font_name).trim().trim_end_matches(';').trim().to_string();
        if let Some(name) = model::kept_font(&name).filter(|n| !n.is_empty())
            && self.fonts.len() < 4096
        {
            self.fonts.insert(self.font_cur, name);
            if let Some(cp) = self.font_cp_cur.take() {
                self.font_cps.insert(self.font_cur, cp);
            }
        }
    }

    /// The code page 8-bit text is read in: the current font's, else the default font's, else the
    /// document's (Windows-1252 when none is named).
    fn code_page(&self) -> u16 {
        self.st.cp.or_else(|| self.font_cps.get(&self.deff).copied()).unwrap_or(if self.ansicpg == 0 { 1252 } else { self.ansicpg })
    }

    /// A byte of 8-bit text (`\'hh` or raw): a single-byte code page decodes it at once; a
    /// multibyte one is fed to a streaming decoder ([`Reader::finish_multibyte`]), so a lead byte decodes
    /// together with its trail byte.
    pub(super) fn byte(&mut self, b: u8) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
            return;
        }
        let cp = self.code_page();
        let Some(encoding) = multibyte_encoding(cp) else {
            self.finish_multibyte();
            let c = wordcraft_doc::encoding::code_page_byte(cp, b);
            self.ch(c);
            return;
        };
        // A code page switch ends the previous run (a lead byte left then becomes U+FFFD, as Word
        // writes it) before the new decoder starts.
        if self.mb_cp != cp || self.mb_decoder.is_none() {
            self.finish_multibyte();
            self.mb_cp = cp;
            self.mb_decoder = Some(encoding.new_decoder_without_bom_handling());
        }
        self.decode_run(&[b]);
    }

    /// Feed 8-bit text bytes to the open multibyte decoder and append what it emits. A single
    /// `\'hh` byte decodes into a stack buffer (no heap allocation per byte of escaped text); a
    /// raw run decodes in one call, so a long ASCII run in a Japanese, Chinese or Korean document
    /// costs one append instead of one per byte. The destination's spare capacity is all
    /// `decode_to_*` writes into, so a full buffer is drained and decoding continues.
    fn decode_run(&mut self, bytes: &[u8]) {
        // The decoder is taken out so the text it produced can be appended (which borrows the
        // reader) before it goes back.
        let Some(mut decoder) = self.mb_decoder.take() else { return };
        let mut read = 0;
        if bytes.len() <= 16 {
            // A character is at most 4 UTF-8 bytes, and a byte may also replace a pending lead
            // byte (3 more), so one escaped byte always fits the stack buffer.
            let mut buf = [0u8; 16];
            let (result, got, written, _) = decoder.decode_to_utf8(bytes, &mut buf, false);
            read = got;
            let full = matches!(result, encoding_rs::CoderResult::OutputFull);
            if written > 0
                && let Ok(s) = std::str::from_utf8(&buf[..written])
            {
                self.text(s);
            }
            if !full {
                self.mb_decoder = Some(decoder);
                return;
            }
        }
        let mut out = String::with_capacity(bytes.len().saturating_mul(2));
        let mut rest = bytes.get(read..).unwrap_or_default();
        loop {
            let (result, got, _) = decoder.decode_to_string(rest, &mut out, false);
            rest = rest.get(got..).unwrap_or_default();
            if !matches!(result, encoding_rs::CoderResult::OutputFull) {
                break;
            }
            out.reserve(16.max(rest.len().saturating_mul(2)));
        }
        self.mb_decoder = Some(decoder);
        if !out.is_empty() {
            self.text(&out);
        }
    }

    /// A run of raw 8-bit text from the file (not `\'hh` escapes): a single-byte code page maps
    /// each byte to a char; a multibyte one decodes the whole run in one pass, so a long ASCII run
    /// in a Japanese, Chinese or Korean document costs one decode and one append instead of one
    /// per byte.
    ///
    /// The decoder is created without BOM handling: the code page comes from `\ansicpg`, `\cpg` or
    /// `\fcharset` and must not change. With BOM sniffing, a run that starts `EF BB BF` in GBK,
    /// Big5 or UHC (a valid character pair followed by a lead byte) would be dropped as a UTF-8
    /// BOM and the rest decoded as UTF-8.
    pub(super) fn run(&mut self, chunk: &[u8]) {
        if self.skip_chars > 0 {
            for b in chunk {
                self.byte(*b);
            }
            return;
        }
        let cp = self.code_page();
        let Some(encoding) = multibyte_encoding(cp) else {
            self.finish_multibyte();
            let mut out = String::with_capacity(chunk.len());
            for b in chunk {
                out.push(wordcraft_doc::encoding::code_page_byte(cp, *b));
            }
            if !out.is_empty() {
                self.text(&out);
            }
            return;
        };
        if self.mb_cp != cp || self.mb_decoder.is_none() {
            self.finish_multibyte();
            self.mb_cp = cp;
            self.mb_decoder = Some(encoding.new_decoder_without_bom_handling());
        }
        self.decode_run(chunk);
    }

    /// Ends the current multibyte run: the streaming decoder emits whatever it still holds (a lone
    /// lead byte becomes U+FFFD, as Word writes it) and is dropped. A code page switch, a single-byte
    /// run, a group end and the end of the document finish the run this way.
    pub(super) fn finish_multibyte(&mut self) {
        let Some(mut decoder) = self.mb_decoder.take() else { return };
        let mut out = String::with_capacity(16);
        let _ = decoder.decode_to_string(&[], &mut out, true);
        if !out.is_empty() {
            self.text(&out);
        }
    }

    pub(super) fn finish_pict(&mut self) {
        if self.pict_half.take().is_some() {
            // Odd hex digit count: drop the dangling nibble.
        }
        let data = std::mem::take(&mut self.pict_hex);
        if self.pict_kind == 0 || data.is_empty() {
            return;
        }
        let sx = if self.pict_sx > 0.0 { self.pict_sx / 100.0 } else { 1.0 };
        let sy = if self.pict_sy > 0.0 { self.pict_sy / 100.0 } else { 1.0 };
        let w = self.pict_w.map(|v| v * sx);
        let h = self.pict_h.map(|v| v * sy);
        if let Some(img) = make_img(data, w, h, "") {
            self.para.inlines.push(Inline::Image(img));
            self.para_has = true;
        }
    }

    pub(super) fn text(&mut self, s: &str) {
        match self.st.dest {
            Dest::Body => {
                if self.st.hidden {
                    return;
                }
                let mut f = self.st.fmt.clone();
                let r = self.st.rev;
                if r.ins {
                    f.ins = Some(self.rev_mark(r.auth, r.dttm));
                }
                if r.del {
                    f.del = Some(self.rev_mark(r.authdel, r.dttmdel));
                }
                let c = &self.st.chg_c;
                if let Some(words) = c.old.clone() {
                    let mark = self.rev_mark(c.auth, c.date);
                    let old = self.old_fmt(&words);
                    f.fmt_change = Some(Box::new(model::FmtChange { mark, old }));
                }
                self.para.push_text(s, &f);
                self.para_has = true;
            }
            Dest::FontTbl => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_font();
                    } else if self.font_name.len() <= model::MAX_FONT_NAME {
                        // One past the limit is enough for `commit_font` to drop the name.
                        self.font_name.push(c);
                    }
                }
            }
            Dest::ColorTbl => {
                for c in s.chars() {
                    if c == ';' {
                        let (r, g, b, set) = self.color_cur;
                        if self.colors.len() < 4096 {
                            self.colors.push(if set { Some(Rgb(r, g, b)) } else { None });
                        }
                        self.color_cur = (0, 0, 0, false);
                    }
                }
            }
            Dest::StyleSheet => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_style();
                    } else if self.style_name.len() < model::MAX_FONT_NAME {
                        self.style_name.push(c);
                    } else {
                        model::dropped(model::Dropped::StyleName);
                    }
                }
            }
            Dest::Info(_) => self.info_text.push_str(s),
            Dest::FldInst => {
                if let Some(f) = self.st.field.and_then(|i| self.fields.get_mut(i))
                    && f.len() < 4096
                {
                    f.push_str(s);
                }
            }
            Dest::Pict => {
                for b in s.bytes() {
                    let v = match b {
                        b'0'..=b'9' => b - b'0',
                        b'a'..=b'f' => b - b'a' + 10,
                        b'A'..=b'F' => b - b'A' + 10,
                        _ => continue,
                    };
                    match self.pict_half.take() {
                        Some(h) => {
                            if self.pict_hex.len() < MAX_PICT {
                                self.pict_hex.push((h << 4) | v);
                            }
                        }
                        None => self.pict_half = Some(v),
                    }
                }
            }
            Dest::Bookmark => {
                if let Some((_, n)) = &mut self.bkmk
                    && n.len() < 256
                {
                    n.push_str(s);
                }
            }
            Dest::RevTbl => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_author();
                    } else if self.rev_name.len() < 256 {
                        self.rev_name.push(c);
                    }
                }
            }
            Dest::Skip | Dest::ListTable | Dest::ListOverride | Dest::OldProps(_) => {}
        }
    }

    /// The formatting before a tracked character change: the current run's, with the
    /// `\oldcprops` words applied (a `\plain` among them starts from none).
    fn old_fmt(&mut self, words: &[(String, Option<i32>)]) -> Fmt {
        let fmt = Fmt { link: None, ins: None, del: None, fmt_change: None, ..self.st.fmt.clone() };
        let saved = std::mem::replace(&mut self.st, St { dest: Dest::OldProps(false), fmt, ..Default::default() });
        for (w, p) in words {
            self.word(w, *p, false, false);
        }
        let old = std::mem::replace(&mut self.st, saved).fmt;
        Fmt { link: None, ..old }
    }

    pub(super) fn ch(&mut self, c: char) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
            return;
        }
        self.finish_multibyte();
        let mut buf = [0u8; 4];
        self.text(c.encode_utf8(&mut buf));
    }
}
