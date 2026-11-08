# Changelog

All notable changes to WordCraft are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- DOCX: Word 2010 custom number formats (`w:numFmt w:val="custom" w:format="…"`, patent-style `[0001]` paragraph numbering) are read and re-emitted on save, so the format round-trips instead of degrading to `[1]` and being stripped (#5318, #5317).
- Control protocol: `list.define` / `list.get` expose each level's `customFormat` — omitted keeps the level's pattern, `null` clears it, an explicit `format` supersedes it; a wrong-typed value is rejected with a parameter error.

### Fixed

- Recognized custom formats render zero-padded like Word's own sample list (`[0001]`, `[0002]`, … from `w:lvlText "[%1]"`), including under a `none` or `bullet` file fallback, while legal numbering (`isLgl`) keeps decimal for every level.
- A numbering tab (Word's "tab stop after number") carries the paragraph text when the position lies inside the first line's span (floats and margins respected); otherwise the ordinary tab stops apply, as in Word's PDF export.
