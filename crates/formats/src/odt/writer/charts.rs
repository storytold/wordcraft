//! Charts written as ODF chart objects (ODF 1.3 §10.4.3 `draw:object` holding an `office:chart`
//! sub-document, Part 3 §11), shaped like LibreOffice's: the chart's data in its own table
//! (`local-table`, labels in the first row and column), each series pointing at its column, its
//! colour the theme accent WordCraft draws it in.

use wordcraft_doc::Rgb;
use wordcraft_doc::chart::{ChartSpec, ChartType, LegendPos};

use super::drawing::{Place, placement};
use super::{Writer, pt, x};
use crate::model;

/// The table's column `i` (from 0) as a cell address names it: `A`, `B`… `Z`, `AA`…
fn column(i: usize) -> String {
    let mut n = i + 1;
    let mut s = Vec::new();
    while n > 0 {
        let d = (n - 1) % 26;
        s.push(b'A' + d as u8);
        n = (n - 1) / 26;
    }
    s.iter().rev().map(|&b| char::from(b)).collect()
}

/// `local-table.$C$2:.$C$5`: column `col`'s rows 2 to `n + 1` (the data under its label).
fn data_range(col: usize, n: usize) -> String {
    let c = column(col);
    format!("local-table.${c}$2:.${c}${}", n + 1)
}

/// Most slices given their own colour (the rest take the chart's own colours).
const MAX_SLICE_STYLES: usize = 64;

/// The `i`th series' (or slice's) colour, `#rrggbb`: the accents in turn.
fn accent(accents: &[Rgb], i: usize) -> String {
    match accents.get(i.checked_rem(accents.len()).unwrap_or(0)) {
        Some(c) => format!("#{}", c.hex().to_ascii_lowercase()),
        None => "#4472c4".into(),
    }
}

/// A number as a table cell's `office:value`: gaps are `NaN`, as LibreOffice writes them.
fn number(v: Option<f64>) -> String {
    match v.filter(|v| v.is_finite()) {
        Some(v) => format!("{v}"),
        None => "NaN".into(),
    }
}

/// A chart sub-document's `content.xml` for `spec`, `w` × `h` points, series in `accents`.
pub(in crate::odt) fn chart_xml(spec: &ChartSpec, w: f32, h: f32, accents: &[Rgb]) -> String {
    let spec = spec.clone().sanitized();
    let (class, series_class) = match spec.kind {
        ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked => ("bar", "bar"),
        ChartType::Line => ("line", "line"),
        ChartType::Area => ("area", "area"),
        ChartType::Pie => ("circle", "circle"),
        ChartType::Doughnut => ("ring", "circle"),
        ChartType::Scatter => ("scatter", "scatter"),
    };
    let vertical = matches!(spec.kind, ChartType::Bar | ChartType::BarStacked);
    let stacked = matches!(spec.kind, ChartType::ColumnStacked | ChartType::BarStacked);
    let labels = if spec.data_labels { " chart:data-label-number=\"value\"" } else { " chart:data-label-number=\"none\"" };
    let n = spec.series.iter().map(|s| s.values.len()).max().unwrap_or(0).max(spec.categories.len());
    // Scatter charts are points without lines.
    let points = if spec.kind == ChartType::Scatter { " chart:lines=\"false\" chart:symbol-type=\"automatic\"" } else { "" };
    // A white wall, labelled axes in grey, a legend without a frame: as WordCraft draws them.
    let mut styles = format!(
        "<style:style style:name=\"plot\" style:family=\"chart\"><style:chart-properties chart:vertical=\"{vertical}\" chart:stacked=\"{stacked}\"{labels}{points}/></style:style>\
         <style:style style:name=\"wall\" style:family=\"chart\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\"/></style:style>\
         <style:style style:name=\"axis\" style:family=\"chart\"><style:chart-properties chart:display-label=\"true\"/><style:graphic-properties svg:stroke-color=\"#b3b3b3\"/><style:text-properties fo:font-size=\"8pt\"/></style:style>\
         <style:style style:name=\"round\" style:family=\"chart\"><style:chart-properties chart:reverse-direction=\"true\"/></style:style>\
         <style:style style:name=\"grid\" style:family=\"chart\"><style:graphic-properties svg:stroke-color=\"#d9d9d9\"/></style:style>\
         <style:style style:name=\"legend\" style:family=\"chart\"><style:graphic-properties draw:stroke=\"none\" draw:fill=\"none\"/><style:text-properties fo:font-size=\"8pt\"/></style:style>"
    );
    // A pie's slices take the accents in turn.
    if spec.kind.is_round() {
        for k in 0..n.min(MAX_SLICE_STYLES) {
            styles.push_str(&format!(
                "<style:style style:name=\"p{k}\" style:family=\"chart\"><style:graphic-properties draw:stroke=\"none\" draw:fill-color=\"{}\"/></style:style>",
                accent(accents, k)
            ));
        }
    }
    let mut series = String::new();
    for i in 0..spec.series.len() {
        let col = i + 1;
        let fill = accent(accents, i);
        let line = match spec.kind {
            ChartType::Line => format!(" draw:stroke=\"solid\" svg:stroke-color=\"{fill}\" svg:stroke-width=\"0.06cm\""),
            _ => " draw:stroke=\"none\"".to_string(),
        };
        styles.push_str(&format!(
            "<style:style style:name=\"s{col}\" style:family=\"chart\"><style:chart-properties{labels}/><style:graphic-properties draw:fill-color=\"{fill}\"{line}/></style:style>"
        ));
        // A scatter series' x values are the first column's.
        let domain = if spec.kind == ChartType::Scatter && n > 0 {
            format!("<chart:domain table:cell-range-address=\"{}\"/>", data_range(0, n))
        } else {
            String::new()
        };
        let values = if n > 0 { format!(" chart:values-cell-range-address=\"{}\"", data_range(col, n)) } else { String::new() };
        let slices: String = if spec.kind.is_round() {
            (0..n.min(MAX_SLICE_STYLES)).map(|k| format!("<chart:data-point chart:style-name=\"p{k}\"/>")).collect()
        } else {
            String::new()
        };
        series.push_str(&format!(
            "<chart:series chart:style-name=\"s{col}\"{values} chart:label-cell-address=\"local-table.${}$1\" chart:class=\"chart:{series_class}\">{domain}{slices}</chart:series>",
            column(col)
        ));
    }
    let title = spec.title.as_deref().map(|t| format!("<chart:title><text:p>{}</text:p></chart:title>", x(t))).unwrap_or_default();
    let legend = spec
        .legend
        .map(|p| {
            let pos = match p {
                LegendPos::Right => "end",
                LegendPos::Left => "start",
                LegendPos::Top => "top",
                LegendPos::Bottom => "bottom",
            };
            format!("<chart:legend chart:legend-position=\"{pos}\" chart:style-name=\"legend\"/>")
        })
        .unwrap_or_default();
    let categories = if n > 0 { format!("<chart:categories table:cell-range-address=\"{}\"/>", data_range(0, n)) } else { String::new() };
    let axes = format!(
        "<chart:axis chart:dimension=\"x\" chart:name=\"primary-x\" chart:style-name=\"axis\">{categories}</chart:axis><chart:axis chart:dimension=\"y\" chart:name=\"primary-y\" chart:style-name=\"{}\">{}</chart:axis>",
        // A pie runs clockwise from the top, as WordCraft draws it.
        if spec.kind.is_round() { "round" } else { "axis" },
        if spec.kind.is_round() { "" } else { "<chart:grid chart:class=\"major\" chart:style-name=\"grid\"/>" }
    );
    // The table: the series' names over their columns, a row per category.
    let mut table =
        String::from("<table:table table:name=\"local-table\"><table:table-header-columns><table:table-column/></table:table-header-columns>");
    if !spec.series.is_empty() {
        table.push_str(&format!(
            "<table:table-columns><table:table-column table:number-columns-repeated=\"{}\"/></table:table-columns>",
            spec.series.len()
        ));
    }
    table.push_str("<table:table-header-rows><table:table-row><table:table-cell><text:p/></table:table-cell>");
    for s in &spec.series {
        table.push_str(&format!("<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>", x(&s.name)));
    }
    table.push_str("</table:table-row></table:table-header-rows><table:table-rows>");
    for k in 0..n {
        let cat = spec.categories.get(k).map(String::as_str).unwrap_or("");
        table.push_str("<table:table-row>");
        // A scatter chart's x values are numbers.
        match cat.trim().parse::<f64>().ok().filter(|v| v.is_finite() && spec.kind == ChartType::Scatter) {
            Some(v) => table.push_str(&format!(
                "<table:table-cell office:value-type=\"float\" office:value=\"{v}\"><text:p>{}</text:p></table:table-cell>",
                x(cat)
            )),
            None => table.push_str(&format!("<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>", x(cat))),
        }
        for s in &spec.series {
            let v = number(s.values.get(k).copied().flatten());
            table.push_str(&format!("<table:table-cell office:value-type=\"float\" office:value=\"{v}\"><text:p>{v}</text:p></table:table-cell>"));
        }
        table.push_str("</table:table-row>");
    }
    table.push_str("</table:table-rows></table:table>");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:chart=\"urn:oasis:names:tc:opendocument:xmlns:chart:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" office:version=\"1.3\"><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:chart><chart:chart svg:width=\"{}\" svg:height=\"{}\" chart:class=\"chart:{class}\">{title}{legend}<chart:plot-area chart:style-name=\"plot\" chart:data-source-has-labels=\"both\">{axes}{series}<chart:wall chart:style-name=\"wall\"/></chart:plot-area>{table}</chart:chart></office:chart></office:body></office:document-content>\n",
        pt(w),
        pt(h)
    )
}

/// Chart object `o` (as read) still reads as chart `spec`: the chart hasn't been edited.
fn unchanged(o: &model::OdfObject, spec: &ChartSpec) -> bool {
    let Some((_, content)) = o.files.iter().find(|(p, _)| p == "content.xml") else { return false };
    let Some(mut read) = super::super::chart::read_chart(content) else { return false };
    read.sanitize();
    let mut spec = spec.clone();
    spec.sanitize();
    read == spec
}

impl Writer {
    /// A chart: its frame here, placed as its float says, and its chart object in the package.
    pub(in crate::odt) fn chart(&mut self, c: &model::Chart, place: Place<'_>, out: &mut String) {
        // Unchanged since it was read: the chart object as it was, with all its look.
        let kept = c.source.as_deref().filter(|o| unchanged(o, &c.spec));
        let (n, inner) = match kept {
            Some(o) => {
                let (n, _, link) = self.kept_object_files(o);
                (n, link)
            }
            None => {
                let (n, name) = self.object_name();
                self.objects.push((name.clone(), chart_xml(&c.spec, c.w, c.h, &self.accents), "application/vnd.oasis.opendocument.chart"));
                let link =
                    format!("<draw:object xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>", x(&name));
                (n, link)
            }
        };
        // Always placed by a style (see `image_frame`).
        let (anchor, props, xy) = placement(place, c.w, c.h);
        let (anchor, style) = (anchor, format!(" draw:style-name=\"{}\"{xy}", self.graphic_style(props)));
        let z = place.z_index();
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Chart{n}\"{style}{anchor} svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"{z}\">{inner}",
            self.prefix,
            pt(c.w),
            pt(c.h),
        ));
        if !c.alt.is_empty() {
            out.push_str(&format!("<svg:desc>{}</svg:desc>", x(&c.alt)));
        }
        out.push_str("</draw:frame>");
    }
}
