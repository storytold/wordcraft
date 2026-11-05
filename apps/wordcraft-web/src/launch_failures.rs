//! The status line for launched files that couldn't be read (`web::launch`).

use wordcraft_ui_egui::i18n;

/// Most failed files one report names on the status bar; it counts the rest.
const MAX_NAMED_FAILURES: usize = 3;

/// The launched files that couldn't be read, for one status line: the reason when one file
/// failed, else a few names and a count (each file's reason is in the log), as Add Font reports.
#[derive(Default)]
pub(crate) struct Failures {
    count: usize,
    names: Vec<String>,
    first_error: String,
}

impl Failures {
    /// Count a file that couldn't be read; the first failure's reason is kept for a lone report.
    pub(crate) fn add(&mut self, name: String, error: String) {
        if self.count == 0 {
            self.first_error = error;
        }
        self.count = self.count.saturating_add(1);
        if self.names.len() < MAX_NAMED_FAILURES {
            self.names.push(name);
        }
    }

    /// The status line, or `None` when every file was read.
    pub(crate) fn report(self) -> Option<String> {
        match (self.count, self.names.first()) {
            (0, _) | (_, None) => None,
            (1, Some(name)) => Some(i18n::fmt(i18n::t("Couldn't open {path}: {error}"), &[("path", name), ("error", &self.first_error)])),
            (count, Some(_)) => {
                let mut names = self.names.join(", ");
                if count > self.names.len() {
                    let more = count.saturating_sub(self.names.len()).to_string();
                    names = i18n::fmt(i18n::t("{names} and {n} more"), &[("names", &names), ("n", &more)]);
                }
                Some(i18n::fmt(i18n::t("{count} files couldn't be opened: {names}."), &[("count", &count.to_string()), ("names", &names)]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(n: usize) -> Option<String> {
        let mut f = Failures::default();
        for i in 1..=n {
            f.add(format!("f{i}.docx"), format!("reason {i}"));
        }
        f.report()
    }

    /// Nothing failed: no status line, so the left-out count (if any) stands alone.
    #[test]
    fn no_failures_report_nothing() {
        assert_eq!(failed(0), None);
    }

    /// One failure says why, as any other open does.
    #[test]
    fn one_failure_gives_its_reason() {
        assert_eq!(failed(1).as_deref(), Some("Couldn't open f1.docx: reason 1"));
    }

    /// Several name the files; past three the rest are counted, so no failure goes unmentioned.
    #[test]
    fn several_failures_are_named_then_counted() {
        assert_eq!(failed(3).as_deref(), Some("3 files couldn't be opened: f1.docx, f2.docx, f3.docx."));
        assert_eq!(failed(5).as_deref(), Some("5 files couldn't be opened: f1.docx, f2.docx, f3.docx and 2 more."));
    }
}
