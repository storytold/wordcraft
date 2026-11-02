use super::{Serialize, Ui, Value, WordApp, json};

/// The Find and Replace dialog's search options beyond the first row (#543).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindFormatSubdialog {
    Font,
    Paragraph,
    Style,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindExtra {
    pub wildcards: bool,
    pub prefix: bool,
    pub suffix: bool,
    pub ignore_punct: bool,
    pub ignore_space: bool,
    /// The search options are shown.
    pub more: bool,
    /// Special ▾ inserts into Replace with (it last had the focus), not Find what.
    pub to_replace: bool,
    pub find_format: Option<wordcraft_doc::CharProps>,
    pub replace_format: Option<wordcraft_doc::CharProps>,
    pub find_style: Option<String>,
    pub replace_style: Option<String>,
    pub find_align: Option<wordcraft_doc::props::Align>,
    pub replace_align: Option<wordcraft_doc::props::Align>,
    pub find_para: Option<wordcraft_doc::props::ParaProps>,
    pub replace_para: Option<wordcraft_doc::props::ParaProps>,
    #[serde(skip)]
    pub subdialog: Option<FindFormatSubdialog>,
    // Font form state:
    #[serde(skip)]
    pub form_font_tab: usize,
    #[serde(skip)]
    pub form_font: String,
    #[serde(skip)]
    pub form_size: String,
    #[serde(skip)]
    pub form_bold: Option<bool>,
    #[serde(skip)]
    pub form_italic: Option<bool>,
    #[serde(skip)]
    pub form_underline: usize,
    #[serde(skip)]
    pub form_strike: Option<bool>,
    #[serde(skip)]
    pub form_double_strike: Option<bool>,
    #[serde(skip)]
    pub form_sup: bool,
    #[serde(skip)]
    pub form_sub: bool,
    #[serde(skip)]
    pub form_small_caps: Option<bool>,
    #[serde(skip)]
    pub form_caps: Option<bool>,
    #[serde(skip)]
    pub form_hidden: Option<bool>,
    #[serde(skip)]
    pub form_color: String,
    #[serde(skip)]
    pub form_highlight: usize,
    #[serde(skip)]
    pub form_spacing: f32,
    #[serde(skip)]
    pub form_spacing_set: bool,
    #[serde(skip)]
    pub form_position: f32,
    #[serde(skip)]
    pub form_position_set: bool,
    #[serde(skip)]
    pub form_kern: f32,
    #[serde(skip)]
    pub form_kern_set: bool,

    // Paragraph form state:
    #[serde(skip)]
    pub form_para_tab: usize,
    #[serde(skip)]
    pub form_align: usize,
    #[serde(skip)]
    pub form_indent_left: f32,
    #[serde(skip)]
    pub form_indent_left_set: bool,
    #[serde(skip)]
    pub form_indent_right: f32,
    #[serde(skip)]
    pub form_indent_right_set: bool,
    #[serde(skip)]
    pub form_indent_special: usize,
    #[serde(skip)]
    pub form_indent_first: f32,
    #[serde(skip)]
    pub form_indent_first_set: bool,
    #[serde(skip)]
    pub form_space_before: f32,
    #[serde(skip)]
    pub form_space_before_set: bool,
    #[serde(skip)]
    pub form_space_after: f32,
    #[serde(skip)]
    pub form_space_after_set: bool,
    #[serde(skip)]
    pub form_line_rule: usize,
    #[serde(skip)]
    pub form_line_val: f32,
    #[serde(skip)]
    pub form_widow: Option<bool>,
    #[serde(skip)]
    pub form_keep_next: Option<bool>,
    #[serde(skip)]
    pub form_keep_lines: Option<bool>,
    #[serde(skip)]
    pub form_page_break: Option<bool>,
    #[serde(skip)]
    pub form_contextual_spacing: Option<bool>,
    #[serde(skip)]
    pub form_mirror_indents: Option<bool>,

    // Style form state:
    #[serde(skip)]
    pub form_style: String,
}

impl FindExtra {
    pub fn open_font_subdialog(&mut self) {
        self.form_font_tab = 0;
        let cur = if self.to_replace { &self.replace_format } else { &self.find_format };
        if let Some(cp) = cur {
            self.form_font = cp.font.clone().unwrap_or_default();
            self.form_size = cp.size.map(|s| s.to_string()).unwrap_or_default();
            self.form_bold = cp.bold;
            self.form_italic = cp.italic;
            self.form_underline = match cp.underline {
                Some(wordcraft_doc::props::Underline::Single) => 1,
                Some(wordcraft_doc::props::Underline::Words) => 2,
                Some(wordcraft_doc::props::Underline::Double) => 3,
                Some(wordcraft_doc::props::Underline::Thick) => 4,
                Some(wordcraft_doc::props::Underline::Dotted) => 5,
                Some(wordcraft_doc::props::Underline::Dash) => 6,
                Some(wordcraft_doc::props::Underline::DotDash) => 7,
                Some(wordcraft_doc::props::Underline::DotDotDash) => 8,
                Some(wordcraft_doc::props::Underline::Wave) => 9,
                Some(wordcraft_doc::props::Underline::DoubleWave) => 10,
                _ => 0,
            };
            self.form_strike = cp.strike;
            self.form_double_strike = cp.double_strike;
            self.form_sup = cp.vert_align == Some(wordcraft_doc::props::VertAlign::Superscript);
            self.form_sub = cp.vert_align == Some(wordcraft_doc::props::VertAlign::Subscript);
            self.form_small_caps = cp.small_caps;
            self.form_caps = cp.caps;
            self.form_hidden = cp.hidden;
            self.form_color = match cp.color {
                Some(wordcraft_doc::props::TextColor::Rgb(rgb)) => rgb.hex(),
                Some(wordcraft_doc::props::TextColor::Auto) => "Auto".into(),
                None => String::new(),
            };
            self.form_highlight = match cp.highlight {
                Some(wordcraft_doc::props::Highlight::Yellow) => 1,
                Some(wordcraft_doc::props::Highlight::BrightGreen) => 2,
                Some(wordcraft_doc::props::Highlight::Turquoise) => 3,
                Some(wordcraft_doc::props::Highlight::Pink) => 4,
                Some(wordcraft_doc::props::Highlight::Blue) => 5,
                Some(wordcraft_doc::props::Highlight::Red) => 6,
                Some(wordcraft_doc::props::Highlight::DarkBlue) => 7,
                Some(wordcraft_doc::props::Highlight::Teal) => 8,
                Some(wordcraft_doc::props::Highlight::Green) => 9,
                Some(wordcraft_doc::props::Highlight::Violet) => 10,
                Some(wordcraft_doc::props::Highlight::DarkRed) => 11,
                Some(wordcraft_doc::props::Highlight::DarkYellow) => 12,
                Some(wordcraft_doc::props::Highlight::Gray50) => 13,
                Some(wordcraft_doc::props::Highlight::Gray25) => 14,
                Some(wordcraft_doc::props::Highlight::Black) => 15,
                Some(wordcraft_doc::props::Highlight::White) => 16,
                _ => 0,
            };
            self.form_spacing = cp.spacing.unwrap_or(0.0);
            self.form_spacing_set = cp.spacing.is_some();
            self.form_position = cp.position.unwrap_or(0.0);
            self.form_position_set = cp.position.is_some();
            self.form_kern = cp.kern.unwrap_or(0.0);
            self.form_kern_set = cp.kern.is_some();
        } else {
            self.form_font.clear();
            self.form_size.clear();
            self.form_bold = None;
            self.form_italic = None;
            self.form_underline = 0;
            self.form_strike = None;
            self.form_double_strike = None;
            self.form_sup = false;
            self.form_sub = false;
            self.form_small_caps = None;
            self.form_caps = None;
            self.form_hidden = None;
            self.form_color.clear();
            self.form_highlight = 0;
            self.form_spacing = 0.0;
            self.form_spacing_set = false;
            self.form_position = 0.0;
            self.form_position_set = false;
            self.form_kern = 0.0;
            self.form_kern_set = false;
        }
        self.subdialog = Some(FindFormatSubdialog::Font);
    }

    pub fn apply_font_subdialog(&mut self) {
        let mut cp = wordcraft_doc::CharProps::default();
        if !self.form_font.trim().is_empty() {
            cp.font = Some(self.form_font.trim().to_string());
        }
        if let Ok(sz) = self.form_size.trim().parse::<f32>()
            && sz > 0.0
        {
            cp.size = Some(sz);
        }
        cp.bold = self.form_bold;
        cp.italic = self.form_italic;
        cp.underline = match self.form_underline {
            1 => Some(wordcraft_doc::props::Underline::Single),
            2 => Some(wordcraft_doc::props::Underline::Words),
            3 => Some(wordcraft_doc::props::Underline::Double),
            4 => Some(wordcraft_doc::props::Underline::Thick),
            5 => Some(wordcraft_doc::props::Underline::Dotted),
            6 => Some(wordcraft_doc::props::Underline::Dash),
            7 => Some(wordcraft_doc::props::Underline::DotDash),
            8 => Some(wordcraft_doc::props::Underline::DotDotDash),
            9 => Some(wordcraft_doc::props::Underline::Wave),
            10 => Some(wordcraft_doc::props::Underline::DoubleWave),
            _ => None,
        };
        cp.strike = self.form_strike;
        cp.double_strike = self.form_double_strike;
        if self.form_sup {
            cp.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
        } else if self.form_sub {
            cp.vert_align = Some(wordcraft_doc::props::VertAlign::Subscript);
        }
        cp.small_caps = self.form_small_caps;
        cp.caps = self.form_caps;
        cp.hidden = self.form_hidden;
        if let Some(tc) = wordcraft_doc::props::TextColor::parse(&self.form_color) {
            cp.color = Some(tc);
        }
        cp.highlight = match self.form_highlight {
            1 => Some(wordcraft_doc::props::Highlight::Yellow),
            2 => Some(wordcraft_doc::props::Highlight::BrightGreen),
            3 => Some(wordcraft_doc::props::Highlight::Turquoise),
            4 => Some(wordcraft_doc::props::Highlight::Pink),
            5 => Some(wordcraft_doc::props::Highlight::Blue),
            6 => Some(wordcraft_doc::props::Highlight::Red),
            7 => Some(wordcraft_doc::props::Highlight::DarkBlue),
            8 => Some(wordcraft_doc::props::Highlight::Teal),
            9 => Some(wordcraft_doc::props::Highlight::Green),
            10 => Some(wordcraft_doc::props::Highlight::Violet),
            11 => Some(wordcraft_doc::props::Highlight::DarkRed),
            12 => Some(wordcraft_doc::props::Highlight::DarkYellow),
            13 => Some(wordcraft_doc::props::Highlight::Gray50),
            14 => Some(wordcraft_doc::props::Highlight::Gray25),
            15 => Some(wordcraft_doc::props::Highlight::Black),
            16 => Some(wordcraft_doc::props::Highlight::White),
            _ => None,
        };
        if self.form_spacing_set {
            cp.spacing = Some(self.form_spacing);
        }
        if self.form_position_set {
            cp.position = Some(self.form_position);
        }
        if self.form_kern_set {
            cp.kern = Some(self.form_kern);
        }
        let is_empty = cp == wordcraft_doc::CharProps::default();
        if self.to_replace {
            self.replace_format = if is_empty { None } else { Some(cp) };
        } else {
            self.find_format = if is_empty { None } else { Some(cp) };
        }
        self.subdialog = None;
    }

    pub fn open_para_subdialog(&mut self) {
        self.form_para_tab = 0;
        let cur_pp = if self.to_replace { &self.replace_para } else { &self.find_para };
        let cur_al = if self.to_replace { self.replace_align } else { self.find_align };
        let al = cur_pp.as_ref().and_then(|p| p.align).or(cur_al);
        self.form_align = match al {
            Some(wordcraft_doc::props::Align::Left) => 1,
            Some(wordcraft_doc::props::Align::Center) => 2,
            Some(wordcraft_doc::props::Align::Right) => 3,
            Some(wordcraft_doc::props::Align::Justify) => 4,
            _ => 0,
        };
        if let Some(pp) = cur_pp {
            self.form_indent_left = pp.indent_left.unwrap_or(0.0);
            self.form_indent_left_set = pp.indent_left.is_some();
            self.form_indent_right = pp.indent_right.unwrap_or(0.0);
            self.form_indent_right_set = pp.indent_right.is_some();
            if let Some(fi) = pp.indent_first {
                self.form_indent_first_set = true;
                if fi >= 0.0 {
                    self.form_indent_special = 1;
                    self.form_indent_first = fi;
                } else {
                    self.form_indent_special = 2;
                    self.form_indent_first = -fi;
                }
            } else {
                self.form_indent_special = 0;
                self.form_indent_first = 0.0;
                self.form_indent_first_set = false;
            }
            self.form_space_before = pp.space_before.unwrap_or(0.0);
            self.form_space_before_set = pp.space_before.is_some();
            self.form_space_after = pp.space_after.unwrap_or(0.0);
            self.form_space_after_set = pp.space_after.is_some();
            match pp.line_spacing {
                Some(wordcraft_doc::props::LineSpacing::Multiple(m)) => {
                    if (m - 1.0).abs() < 0.05 {
                        self.form_line_rule = 1;
                        self.form_line_val = 1.0;
                    } else if (m - 1.5).abs() < 0.05 {
                        self.form_line_rule = 2;
                        self.form_line_val = 1.5;
                    } else if (m - 2.0).abs() < 0.05 {
                        self.form_line_rule = 3;
                        self.form_line_val = 2.0;
                    } else {
                        self.form_line_rule = 6;
                        self.form_line_val = m;
                    }
                }
                Some(wordcraft_doc::props::LineSpacing::AtLeast(pt)) => {
                    self.form_line_rule = 4;
                    self.form_line_val = pt;
                }
                Some(wordcraft_doc::props::LineSpacing::Exactly(pt)) => {
                    self.form_line_rule = 5;
                    self.form_line_val = pt;
                }
                _ => {
                    self.form_line_rule = 0;
                    self.form_line_val = 1.0;
                }
            }
            self.form_widow = pp.widow_control;
            self.form_keep_next = pp.keep_next;
            self.form_keep_lines = pp.keep_lines;
            self.form_page_break = pp.page_break_before;
            self.form_contextual_spacing = pp.contextual_spacing;
            self.form_mirror_indents = pp.mirror_indents;
        } else {
            self.form_indent_left = 0.0;
            self.form_indent_left_set = false;
            self.form_indent_right = 0.0;
            self.form_indent_right_set = false;
            self.form_indent_special = 0;
            self.form_indent_first = 0.0;
            self.form_indent_first_set = false;
            self.form_space_before = 0.0;
            self.form_space_before_set = false;
            self.form_space_after = 0.0;
            self.form_space_after_set = false;
            self.form_line_rule = 0;
            self.form_line_val = 1.0;
            self.form_widow = None;
            self.form_keep_next = None;
            self.form_keep_lines = None;
            self.form_page_break = None;
            self.form_contextual_spacing = None;
            self.form_mirror_indents = None;
        }
        self.subdialog = Some(FindFormatSubdialog::Paragraph);
    }

    pub fn apply_para_subdialog(&mut self) {
        let mut pp = wordcraft_doc::ParaProps::default();
        let al = match self.form_align {
            1 => Some(wordcraft_doc::props::Align::Left),
            2 => Some(wordcraft_doc::props::Align::Center),
            3 => Some(wordcraft_doc::props::Align::Right),
            4 => Some(wordcraft_doc::props::Align::Justify),
            _ => None,
        };
        pp.align = al;
        if self.form_indent_left_set {
            pp.indent_left = Some(self.form_indent_left);
        }
        if self.form_indent_right_set {
            pp.indent_right = Some(self.form_indent_right);
        }
        if self.form_indent_first_set && self.form_indent_special > 0 {
            if self.form_indent_special == 1 {
                pp.indent_first = Some(self.form_indent_first.abs());
            } else if self.form_indent_special == 2 {
                pp.indent_first = Some(-self.form_indent_first.abs());
            }
        }
        if self.form_space_before_set {
            pp.space_before = Some(self.form_space_before);
        }
        if self.form_space_after_set {
            pp.space_after = Some(self.form_space_after);
        }
        pp.line_spacing = match self.form_line_rule {
            1 => Some(wordcraft_doc::props::LineSpacing::Multiple(1.0)),
            2 => Some(wordcraft_doc::props::LineSpacing::Multiple(1.5)),
            3 => Some(wordcraft_doc::props::LineSpacing::Multiple(2.0)),
            4 => Some(wordcraft_doc::props::LineSpacing::AtLeast(self.form_line_val)),
            5 => Some(wordcraft_doc::props::LineSpacing::Exactly(self.form_line_val)),
            6 => Some(wordcraft_doc::props::LineSpacing::Multiple(self.form_line_val)),
            _ => None,
        };
        pp.widow_control = self.form_widow;
        pp.keep_next = self.form_keep_next;
        pp.keep_lines = self.form_keep_lines;
        pp.page_break_before = self.form_page_break;
        pp.contextual_spacing = self.form_contextual_spacing;
        pp.mirror_indents = self.form_mirror_indents;

        let is_empty = pp == wordcraft_doc::ParaProps::default();
        if self.to_replace {
            self.replace_align = al;
            self.replace_para = if is_empty { None } else { Some(pp) };
        } else {
            self.find_align = al;
            self.find_para = if is_empty { None } else { Some(pp) };
        }
        self.subdialog = None;
    }

    pub fn open_style_subdialog(&mut self) {
        let cur = if self.to_replace { &self.replace_style } else { &self.find_style };
        self.form_style = cur.clone().unwrap_or_default();
        self.subdialog = Some(FindFormatSubdialog::Style);
    }

    pub fn apply_style_subdialog(&mut self) {
        let st = if self.form_style.trim().is_empty() { None } else { Some(self.form_style.trim().to_string()) };
        if self.to_replace {
            self.replace_style = st;
        } else {
            self.find_style = st;
        }
        self.subdialog = None;
    }
}

/// The tabs of the Find and Replace dialog (#712).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindTab {
    #[default]
    Find,
    Replace,
    GoTo,
}

/// What Go To moves to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GoToWhat {
    #[default]
    Page,
    Bookmark,
}

/// The Special ▾ menu: (name, code) for Find what, without and with wildcards.
const SPECIAL_FIND: &[(&str, &str)] = &[
    ("Paragraph Mark", "^p"),
    ("Tab Character", "^t"),
    ("Any Character", "^?"),
    ("Any Digit", "^#"),
    ("Any Letter", "^$"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Graphic", "^g"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
    ("Section Break", "^b"),
    ("White Space", "^w"),
];

const SPECIAL_FIND_WILD: &[(&str, &str)] = &[
    ("Any Character", "?"),
    ("Character in Range", "[-]"),
    ("Beginning of Word", "<"),
    ("End of Word", ">"),
    ("Expression", "()"),
    ("Not", "[!]"),
    ("Num Occurrences", "{,}"),
    ("Previous 1 or More", "@"),
    ("0 or More Characters", "*"),
    ("Paragraph Mark", "^13"),
    ("Tab Character", "^t"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Graphic", "^g"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
];

/// Replace with; with wildcards, "Find What Expression" (`\1`) comes first.
const SPECIAL_REPLACE: &[(&str, &str)] = &[
    ("Paragraph Mark", "^p"),
    ("Tab Character", "^t"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Clipboard Contents", "^c"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Find What Text", "^&"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
];

/// The Special ▾ menu button; returns the code picked.
pub(super) fn special_menu(ui: &mut Ui, extra: &FindExtra) -> Option<&'static str> {
    let mut picked = None;
    ui.menu_button(format!("{} ▾", tl!("Special")), |ui| {
        let wild_replace: &[(&str, &str)] = &[("Find What Expression", "\\1")];
        let lists: [&[(&str, &str)]; 2] = match (extra.to_replace, extra.wildcards) {
            (false, false) => [SPECIAL_FIND, &[]],
            (false, true) => [SPECIAL_FIND_WILD, &[]],
            (true, false) => [SPECIAL_REPLACE, &[]],
            (true, true) => [wild_replace, SPECIAL_REPLACE],
        };
        for (name, code) in lists.into_iter().flatten() {
            if ui.button(format!("{}   {code}", tl!(name))).clicked() {
                picked = Some(*code);
                ui.close();
            }
        }
    });
    picked
}

/// What Search lists before anything is typed: common commands (#819).
pub(super) const SEARCH_SUGGESTIONS: [&str; 3] = ["review.spelling", "review.wordCount", "file.print"];

/// Search's "Find … in the document": the Navigation pane opens on its results for `text`.
pub(super) fn find_in_navigation_pane(app: &mut WordApp, ctx: &egui::Context, text: &str) {
    let qid = app.window_id("nav_query");
    ctx.data_mut(|d| d.insert_temp(qid, text.to_string()));
    app.ui.nav_tab = "results".into();
    let _ = app.run("view.navigationPane", json!({"value": true}));
    let _ = app.run("edit.find", json!({"text": text}));
}

/// Whether `e` is the text of the shortcut that opened a box (see `Dialog::Commands::swallow`):
/// a plain key press first means the user is typing, and one swallowed text is all there is.
pub(super) fn swallow_shortcut_text(swallow: &mut bool, e: &egui::Event) -> bool {
    match e {
        egui::Event::Key { pressed: true, modifiers, .. } if !modifiers.alt => {
            *swallow = false;
            false
        }
        egui::Event::Text(_) if *swallow => {
            *swallow = false;
            true
        }
        _ => false,
    }
}

/// Search's filter: every word of the query is in the command's label, location or id, in English
/// or in the interface language, so "insert table" finds Insert › Tables › Table (#819).
pub(super) fn command_matches(c: &wordcraft_engine::CommandSpec, query: &str) -> bool {
    if query.trim().is_empty() {
        return false;
    }
    let hay = format!(
        "{} {} {} {} {}",
        c.label.to_lowercase(),
        c.location.to_lowercase(),
        c.id.to_lowercase(),
        crate::i18n::lowercase(tl!(c.label)),
        crate::i18n::lowercase(&crate::i18n::location(c.location))
    );
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()) || hay.contains(&crate::i18n::lowercase(w)))
}

/// The Find and Replace dialog's Go To tab (#712): a page (a number, or `+2` / `-1` from the
/// current page; empty, Next and Previous step a page) or a bookmark. Returns true to close.
pub(super) fn goto_body(app: &mut WordApp, ui: &mut Ui, what: &mut GoToWhat, target: &mut String, message: &mut String) -> bool {
    // The document's bookmarks, only while they are listed (finding them walks the document).
    let bookmarks: Vec<String> = if *what == GoToWhat::Bookmark {
        app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect()
    } else {
        Vec::new()
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Go to what:"));
            for (w, label) in [(GoToWhat::Page, "Page"), (GoToWhat::Bookmark, "Bookmark")] {
                if ui.selectable_label(*what == w, tl!(label)).clicked() && *what != w {
                    *what = w;
                    target.clear();
                    message.clear();
                }
            }
        });
        ui.add_space(16.0);
        ui.vertical(|ui| match what {
            GoToWhat::Page => {
                let label = ui.label(tl!("Enter page number:"));
                let r = ui.add(egui::TextEdit::singleline(target).desired_width(220.0)).labelled_by(label.id);
                if !r.has_focus() {
                    r.request_focus();
                }
                ui.label(egui::RichText::new(tl!("Type +2 or -1 to move from the page you are on.")).weak());
            }
            GoToWhat::Bookmark => {
                let label = ui.label(tl!("Enter bookmark name:"));
                craft_ui::ComboBox::from_id_salt("goto_bookmark")
                    .width(220.0)
                    .selected_text(target.as_str())
                    .show_ui(ui, |ui| {
                        for b in &bookmarks {
                            ui.selectable_value(target, b.clone(), b.as_str());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if bookmarks.is_empty() {
                    ui.label(egui::RichText::new(tl!("This document has no bookmarks.")).weak());
                }
            }
        });
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).weak());
    }
    let mut close = false;
    let mut go: Option<Value> = None;
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let page_step = *what == GoToWhat::Page && target.trim().is_empty();
        if page_step && ui.button(tl!("Previous")).clicked() {
            go = Some(json!({"pageBy": -1}));
        }
        let label = if page_step { tl!("Next") } else { tl!("Go To") };
        if ui.button(label).clicked() || enter {
            go = match what {
                GoToWhat::Page => match goto_page(target) {
                    Some(p) => Some(p),
                    None => {
                        *message = tl!("Type a page number, or +2 / -1.").to_string();
                        None
                    }
                },
                GoToWhat::Bookmark if !target.is_empty() => Some(json!({"bookmark": target.clone()})),
                GoToWhat::Bookmark => None,
            };
        }
        if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if let Some(params) = go {
        match app.run("edit.goto", params) {
            Ok(_) => message.clear(),
            Err(e) => *message = e,
        }
    }
    close
}

/// Go To's page box as `edit.goto` params: empty steps one page on, `+n` / `-n` move from the
/// current page, a number is that page. `None` for anything else.
pub(super) fn goto_page(text: &str) -> Option<Value> {
    let t = text.trim().replace('\u{2212}', "-");
    if t.is_empty() {
        return Some(json!({"pageBy": 1}));
    }
    if let Some(rest) = t.strip_prefix('+') {
        return rest.parse::<i64>().ok().filter(|n| *n >= 0).map(|n| json!({"pageBy": n}));
    }
    if t.starts_with('-') {
        return t.parse::<i64>().ok().map(|n| json!({"pageBy": n}));
    }
    t.parse::<u64>().ok().filter(|n| *n >= 1).map(|n| json!({"page": n}))
}

fn tri_state_checkbox(ui: &mut egui::Ui, state: &mut Option<bool>, text: &str) {
    let mut checked = state.unwrap_or(false);
    let indeterminate = state.is_none();
    let cb = egui::Checkbox::new(&mut checked, text).indeterminate(indeterminate);
    if ui.add(cb).clicked() {
        *state = match *state {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
    }
}

fn char_props_effects_desc(cp: &wordcraft_doc::CharProps, parts: &mut Vec<String>) {
    if cp.strike == Some(true) {
        parts.push("Strikethrough".into());
    }
    if cp.double_strike == Some(true) {
        parts.push("Double strikethrough".into());
    }
    if cp.vert_align == Some(wordcraft_doc::props::VertAlign::Superscript) {
        parts.push("Superscript".into());
    } else if cp.vert_align == Some(wordcraft_doc::props::VertAlign::Subscript) {
        parts.push("Subscript".into());
    }
    if cp.small_caps == Some(true) {
        parts.push("Small caps".into());
    }
    if cp.caps == Some(true) {
        parts.push("All caps".into());
    }
    if cp.hidden == Some(true) {
        parts.push("Hidden".into());
    }
    if let Some(c) = &cp.color {
        match c {
            wordcraft_doc::props::TextColor::Rgb(rgb) => parts.push(format!("Color: {}", rgb.hex())),
            wordcraft_doc::props::TextColor::Auto => parts.push("Color: Auto".into()),
        }
    }
    if let Some(hl) = &cp.highlight {
        parts.push(format!("Highlight: {hl:?}"));
    }
    if let Some(sp) = cp.spacing {
        parts.push(format!("Spacing: {sp} pt"));
    }
    if let Some(k) = cp.kern {
        parts.push(format!("Kern: {k} pt"));
    }
}

fn char_props_description(cp: &wordcraft_doc::CharProps) -> String {
    let mut parts = Vec::new();
    if let Some(font) = &cp.font {
        parts.push(font.clone());
    }
    if let Some(sz) = cp.size {
        parts.push(format!("{sz} pt"));
    }
    if cp.bold == Some(true) {
        parts.push("Bold".into());
    } else if cp.bold == Some(false) {
        parts.push("Not Bold".into());
    }
    if cp.italic == Some(true) {
        parts.push("Italic".into());
    } else if cp.italic == Some(false) {
        parts.push("Not Italic".into());
    }
    if let Some(u) = cp.underline
        && u != wordcraft_doc::props::Underline::None
    {
        parts.push(format!("Underline: {u:?}"));
    }
    char_props_effects_desc(cp, &mut parts);
    if parts.is_empty() { String::new() } else { format!("Font: {}", parts.join(", ")) }
}

fn para_props_description(pp: &wordcraft_doc::props::ParaProps) -> String {
    let mut parts = Vec::new();
    if let Some(l) = pp.indent_left {
        parts.push(format!("Indent Left: {l} pt"));
    }
    if let Some(r) = pp.indent_right {
        parts.push(format!("Indent Right: {r} pt"));
    }
    if let Some(f) = pp.indent_first {
        if f >= 0.0 {
            parts.push(format!("First line: {f} pt"));
        } else {
            parts.push(format!("Hanging: {} pt", -f));
        }
    }
    if let Some(b) = pp.space_before {
        parts.push(format!("Before: {b} pt"));
    }
    if let Some(a) = pp.space_after {
        parts.push(format!("After: {a} pt"));
    }
    if let Some(ls) = pp.line_spacing {
        match ls {
            wordcraft_doc::props::LineSpacing::Multiple(m) => parts.push(format!("Line spacing: {m} lines")),
            wordcraft_doc::props::LineSpacing::AtLeast(pt) => parts.push(format!("Line spacing: at least {pt} pt")),
            wordcraft_doc::props::LineSpacing::Exactly(pt) => parts.push(format!("Line spacing: exactly {pt} pt")),
        }
    }
    if pp.page_break_before == Some(true) {
        parts.push("Page break before".into());
    }
    if pp.keep_next == Some(true) {
        parts.push("Keep with next".into());
    }
    if pp.keep_lines == Some(true) {
        parts.push("Keep lines together".into());
    }
    if pp.widow_control == Some(true) {
        parts.push("Widow/Orphan control".into());
    }
    if parts.is_empty() { String::new() } else { format!("Paragraph: {}", parts.join(", ")) }
}

pub(super) fn format_description(
    cp: Option<&wordcraft_doc::CharProps>,
    style: Option<&str>,
    align: Option<wordcraft_doc::props::Align>,
    pp: Option<&wordcraft_doc::props::ParaProps>,
) -> String {
    let mut parts = Vec::new();
    if let Some(cp) = cp {
        let desc = char_props_description(cp);
        if !desc.is_empty() {
            parts.push(desc);
        }
    }
    if let Some(st) = style {
        parts.push(format!("Style: {st}"));
    }
    let effective_al = align.or(pp.and_then(|p| p.align));
    if let Some(al) = effective_al {
        let s = match al {
            wordcraft_doc::props::Align::Left => "Left",
            wordcraft_doc::props::Align::Center => "Center",
            wordcraft_doc::props::Align::Right => "Right",
            wordcraft_doc::props::Align::Justify => "Justified",
            _ => "Other",
        };
        parts.push(format!("Alignment: {s}"));
    }
    if let Some(pp) = pp {
        let desc = para_props_description(pp);
        if !desc.is_empty() {
            parts.push(desc);
        }
    }
    if parts.is_empty() { String::new() } else { format!("Format: {}", parts.join("; ")) }
}

pub fn show_format_subdialogs(ctx: &egui::Context, extra: &mut FindExtra, doc_styles: &[String], font_families: &[String]) {
    match extra.subdialog {
        Some(FindFormatSubdialog::Font) => {
            let mut close = false;
            let title = if extra.to_replace { tl!("Replace Font") } else { tl!("Find Font") };
            egui::Window::new(title).collapsible(false).resizable(false).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut extra.form_font_tab, 0, tl!("Font"));
                    ui.selectable_value(&mut extra.form_font_tab, 1, tl!("Advanced"));
                });
                ui.separator();
                if extra.form_font_tab == 0 {
                    egui::Grid::new("find_font_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(tl!("Font:"));
                        let shown = if extra.form_font.is_empty() { tl!("(Any)") } else { extra.form_font.as_str() };
                        craft_ui::ComboBox::from_id_salt("find_font_fam").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut extra.form_font, String::new(), tl!("(Any)"));
                            for f in font_families {
                                ui.selectable_value(&mut extra.form_font, f.clone(), f);
                            }
                        });
                        ui.end_row();

                        ui.label(tl!("Size:"));
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut extra.form_size);
                            ui.label(tl!("pt (blank for any)"));
                        });
                        ui.end_row();

                        ui.label(tl!("Font style:"));
                        ui.horizontal(|ui| {
                            tri_state_checkbox(ui, &mut extra.form_bold, tl!("Bold"));
                            tri_state_checkbox(ui, &mut extra.form_italic, tl!("Italic"));
                        });
                        ui.end_row();

                        ui.label(tl!("Underline:"));
                        let u_labels = [
                            tl!("(None)"),
                            tl!("Single"),
                            tl!("Words only"),
                            tl!("Double"),
                            tl!("Thick"),
                            tl!("Dotted"),
                            tl!("Dash"),
                            tl!("DotDash"),
                            tl!("DotDotDash"),
                            tl!("Wave"),
                            tl!("DoubleWave"),
                        ];
                        craft_ui::ComboBox::from_id_salt("find_font_u")
                            .selected_text(u_labels.get(extra.form_underline).copied().unwrap_or(tl!("(None)")))
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                for (i, l) in u_labels.iter().enumerate() {
                                    ui.selectable_value(&mut extra.form_underline, i, *l);
                                }
                            });
                        ui.end_row();

                        ui.label(tl!("Font color:"));
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut extra.form_color);
                            ui.label(tl!("(#RRGGBB or Auto)"));
                        });
                        ui.end_row();

                        ui.label(tl!("Highlight:"));
                        let hl_labels = [
                            tl!("(None)"),
                            tl!("Yellow"),
                            tl!("BrightGreen"),
                            tl!("Turquoise"),
                            tl!("Pink"),
                            tl!("Blue"),
                            tl!("Red"),
                            tl!("DarkBlue"),
                            tl!("Teal"),
                            tl!("Green"),
                            tl!("Violet"),
                            tl!("DarkRed"),
                            tl!("DarkYellow"),
                            tl!("Gray50"),
                            tl!("Gray25"),
                            tl!("Black"),
                            tl!("White"),
                        ];
                        craft_ui::ComboBox::from_id_salt("find_font_hl")
                            .selected_text(hl_labels.get(extra.form_highlight).copied().unwrap_or(tl!("(None)")))
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                for (i, l) in hl_labels.iter().enumerate() {
                                    ui.selectable_value(&mut extra.form_highlight, i, *l);
                                }
                            });
                        ui.end_row();
                    });

                    ui.separator();
                    ui.label(egui::RichText::new(tl!("Effects")).strong());
                    egui::Grid::new("find_font_fx").num_columns(2).spacing([20.0, 6.0]).show(ui, |ui| {
                        tri_state_checkbox(ui, &mut extra.form_strike, tl!("Strikethrough"));
                        tri_state_checkbox(ui, &mut extra.form_double_strike, tl!("Double strikethrough"));
                        ui.end_row();

                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut extra.form_sup, tl!("Superscript")).clicked() && extra.form_sup {
                                extra.form_sub = false;
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut extra.form_sub, tl!("Subscript")).clicked() && extra.form_sub {
                                extra.form_sup = false;
                            }
                        });
                        ui.end_row();

                        tri_state_checkbox(ui, &mut extra.form_small_caps, tl!("Small caps"));
                        tri_state_checkbox(ui, &mut extra.form_caps, tl!("All caps"));
                        ui.end_row();

                        tri_state_checkbox(ui, &mut extra.form_hidden, tl!("Hidden"));
                        ui.end_row();
                    });
                } else {
                    egui::Grid::new("find_font_adv").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                        ui.label(tl!("Character Spacing:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_spacing_set, "");
                            ui.add_enabled(extra.form_spacing_set, egui::DragValue::new(&mut extra.form_spacing).speed(0.1).range(-20.0..=20.0));
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("Position:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_position_set, "");
                            ui.add_enabled(extra.form_position_set, egui::DragValue::new(&mut extra.form_position).speed(0.1).range(-20.0..=20.0));
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("Kerning for fonts:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_kern_set, "");
                            ui.add_enabled(extra.form_kern_set, egui::DragValue::new(&mut extra.form_kern).speed(0.5).range(1.0..=1638.0));
                            ui.label(tl!("Points and above"));
                        });
                        ui.end_row();
                    });
                }

                ui.separator();
                ui.label(egui::RichText::new(tl!("Preview")).small().weak());
                let preview_sz = extra.form_size.trim().parse::<f32>().unwrap_or(12.0).clamp(8.0, 24.0);
                let preview_color = wordcraft_doc::Rgb::parse(&extra.form_color).map(crate::theme::c32).unwrap_or_else(|| ui.visuals().text_color());
                let mut fmt = egui::TextFormat {
                    font_id: egui::FontId::new(preview_sz, egui::FontFamily::Proportional),
                    color: preview_color,
                    ..Default::default()
                };
                if extra.form_bold == Some(true) {
                    fmt.font_id = crate::theme::semibold(preview_sz);
                }
                if extra.form_italic == Some(true) {
                    fmt.italics = true;
                }
                if extra.form_underline > 0 {
                    fmt.underline = egui::Stroke::new(1.0, preview_color);
                }
                if extra.form_strike == Some(true) || extra.form_double_strike == Some(true) {
                    fmt.strikethrough = egui::Stroke::new(1.0, preview_color);
                }
                let preview_sample = if extra.form_caps == Some(true) { "AABBYYZZ 123" } else { "AaBbYyZz 123" };
                let job = egui::text::LayoutJob::single_section(preview_sample.into(), fmt);
                egui::Frame::canvas(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_height(36.0);
                    ui.centered_and_justified(|ui| {
                        ui.label(job);
                    });
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("OK")).clicked() {
                        extra.apply_font_subdialog();
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                extra.subdialog = None;
            }
        }
        Some(FindFormatSubdialog::Paragraph) => {
            let mut close = false;
            let title = if extra.to_replace { tl!("Replace Paragraph") } else { tl!("Find Paragraph") };
            egui::Window::new(title).collapsible(false).resizable(false).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_min_width(400.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut extra.form_para_tab, 0, tl!("Indents and Spacing"));
                    ui.selectable_value(&mut extra.form_para_tab, 1, tl!("Line and Page Breaks"));
                });
                ui.separator();
                if extra.form_para_tab == 0 {
                    ui.label(egui::RichText::new(tl!("General")).strong());
                    egui::Grid::new("find_para_gen").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(tl!("Alignment:"));
                        let al_labels = [tl!("(Any)"), tl!("Left"), tl!("Center"), tl!("Right"), tl!("Justified")];
                        craft_ui::ComboBox::from_id_salt("find_para_al")
                            .selected_text(al_labels.get(extra.form_align).copied().unwrap_or(tl!("(Any)")))
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                for (i, l) in al_labels.iter().enumerate() {
                                    ui.selectable_value(&mut extra.form_align, i, *l);
                                }
                            });
                        ui.end_row();
                    });

                    ui.separator();
                    ui.label(egui::RichText::new(tl!("Indentation")).strong());
                    egui::Grid::new("find_para_ind").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(tl!("Left indent:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_indent_left_set, "");
                            ui.add_enabled(extra.form_indent_left_set, egui::DragValue::new(&mut extra.form_indent_left).speed(1.0));
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("Right indent:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_indent_right_set, "");
                            ui.add_enabled(extra.form_indent_right_set, egui::DragValue::new(&mut extra.form_indent_right).speed(1.0));
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("Special:"));
                        ui.horizontal(|ui| {
                            let sp_labels = [tl!("(None)"), tl!("First line"), tl!("Hanging")];
                            craft_ui::ComboBox::from_id_salt("find_para_sp_ind")
                                .selected_text(sp_labels.get(extra.form_indent_special).copied().unwrap_or(tl!("(None)")))
                                .width(120.0)
                                .show_ui(ui, |ui| {
                                    for (i, l) in sp_labels.iter().enumerate() {
                                        if ui.selectable_value(&mut extra.form_indent_special, i, *l).clicked() {
                                            extra.form_indent_first_set = i > 0;
                                        }
                                    }
                                });
                            if extra.form_indent_special > 0 {
                                ui.label(tl!("By:"));
                                ui.add(egui::DragValue::new(&mut extra.form_indent_first).speed(1.0).range(0.0..=500.0));
                                ui.label(tl!("pt"));
                            }
                        });
                        ui.end_row();
                    });
                    tri_state_checkbox(ui, &mut extra.form_mirror_indents, tl!("Mirror indents"));

                    ui.separator();
                    ui.label(egui::RichText::new(tl!("Spacing")).strong());
                    egui::Grid::new("find_para_sp").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(tl!("Before:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_space_before_set, "");
                            ui.add_enabled(
                                extra.form_space_before_set,
                                egui::DragValue::new(&mut extra.form_space_before).speed(1.0).range(0.0..=1584.0),
                            );
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("After:"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut extra.form_space_after_set, "");
                            ui.add_enabled(
                                extra.form_space_after_set,
                                egui::DragValue::new(&mut extra.form_space_after).speed(1.0).range(0.0..=1584.0),
                            );
                            ui.label(tl!("pt"));
                        });
                        ui.end_row();

                        ui.label(tl!("Line spacing:"));
                        ui.horizontal(|ui| {
                            let ls_labels =
                                [tl!("(Any)"), tl!("Single"), tl!("1.5 lines"), tl!("Double"), tl!("At least"), tl!("Exactly"), tl!("Multiple")];
                            craft_ui::ComboBox::from_id_salt("find_para_ls")
                                .selected_text(ls_labels.get(extra.form_line_rule).copied().unwrap_or(tl!("(Any)")))
                                .width(120.0)
                                .show_ui(ui, |ui| {
                                    for (i, l) in ls_labels.iter().enumerate() {
                                        ui.selectable_value(&mut extra.form_line_rule, i, *l);
                                    }
                                });
                            if extra.form_line_rule >= 4 {
                                ui.label(tl!("At:"));
                                ui.add(egui::DragValue::new(&mut extra.form_line_val).speed(0.1).range(0.5..=100.0));
                            }
                        });
                        ui.end_row();
                    });
                    tri_state_checkbox(ui, &mut extra.form_contextual_spacing, tl!("Don't add space between paragraphs of the same style"));
                } else {
                    ui.label(egui::RichText::new(tl!("Pagination")).strong());
                    tri_state_checkbox(ui, &mut extra.form_widow, tl!("Widow/Orphan control"));
                    tri_state_checkbox(ui, &mut extra.form_keep_next, tl!("Keep with next"));
                    tri_state_checkbox(ui, &mut extra.form_keep_lines, tl!("Keep lines together"));
                    tri_state_checkbox(ui, &mut extra.form_page_break, tl!("Page break before"));
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("OK")).clicked() {
                        extra.apply_para_subdialog();
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                extra.subdialog = None;
            }
        }
        Some(FindFormatSubdialog::Style) => {
            let mut close = false;
            let title = if extra.to_replace { tl!("Replace Style") } else { tl!("Find Style") };
            egui::Window::new(title).collapsible(false).resizable(false).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_min_width(280.0);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl!("Style:")).strong());
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for st in doc_styles {
                        ui.selectable_value(&mut extra.form_style, st.clone(), st);
                    }
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("OK")).clicked() {
                        extra.apply_style_subdialog();
                    }
                    if ui.button(tl!("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close {
                extra.subdialog = None;
            }
        }
        None => {}
    }
}
