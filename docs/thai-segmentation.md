# Thai dictionary line breaking

Status: 2026-11-06. Scope: issue #1548, wrapping only.

Thai paragraphs use ICU4X `LineSegmenter::new_dictionary` with `icu_segmenter =2.3.0`,
default features disabled and only `compiled_data` enabled. The dictionary is embedded;
there is no network lookup or LSTM feature. Other scripts retain the existing line-break rules.
Mapped visible-text boundaries and emergency breaks preserve safe grapheme/shaping units.

Thai justification, Thai Distributed model/commands and format mappings are reviewed in a
separate change. Recent-file fallback fonts are already part of the base branch.

The dependency and its data are licensed under Unicode-3.0; see
`licenses/ICU4X-Unicode-3.0.txt`. Suite-wide dependency acceptance and agreement on a common
version remain a maintainer decision. This PR pins its local version without claiming that
decision has been made. Full desktop binary growth and cross-application Word parity remain
unverified.
