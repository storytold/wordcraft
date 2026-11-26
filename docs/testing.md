# Layout test fonts

> **Last reviewed:** 2026-11-23 · **Last updated:** 2026-11-23 · **Change:** trivial (isolated Myanmar shaping fixture, #4314)

Layout unit tests use only the 13 font files already pinned under `assets/fonts/`: Source Sans 3,
Source Serif 4, Inter and JetBrains Mono, with their bundled styles. Family listings also include
legacy style names such as Inter Medium. They do not load installed fonts, user font preferences,
or the optional `CRAFT_FONTS_DIR` input. No download or extra font checkout is required.

Layout entry points (`layout`, `layout_para`, `layout_equation` and
`display::mark_text_width`) initialize the process-wide font database once in unit-test builds,
including when a single test is run by name. Tests that resolve fonts before calling a layout
entry point must first call `crate::test_fonts();`. The `wordcraft-fonts/test-support`
development feature exposes the initializer; enabling the feature alone has no effect.
Initialization fails if an application database already exists, rather than replacing a database
whose faces may be in the shaping or math caches. Other test binaries and application builds
keep their normal font discovery behavior.

Uninstalled requested families use the normal substitution lists, restricted to this fixed set.
In particular, the default Aptos request resolves to Source Sans 3. These fonts are not Calibri
metric substitutes: Word measurements remain reference behavior, while fixture dimensions must
account for the pinned font's metrics. Do not recalibrate a fixture from the same layout behavior
it is meant to test. When changing a pagination fixture, check that undoing its original layout
fix still makes the regression test fail.

The fixed set does not cover every script or include an OpenType math font. Tests with an
existing unavailable-script guard still take that guard. Additional script or math font fixtures
belong in `storytold/craft-fonts`, with a reproducible test input before enabling them here.

Run the suite with `cargo test -p wordcraft-layout`. The isolated font initialization regression
runs with `cargo test -p wordcraft-fonts --features test-support --test fixed_test_fonts`.
`cargo xtask ci` also runs it through the layout crate's development dependency.

## Explicit Myanmar shaping regression

The direct Myanmar reordering test accepts `WORDCRAFT_TEST_MYANMAR_FONT`, pointing to
Noto Sans Myanmar Regular. It loads that face into an isolated font database and validates its
family, coverage and layout tables; it does not add it to layout’s fixed global font set. Linux CI
installs `fonts-noto-core` and supplies the path. No font binary is added to this repository.

## Revision history

- 2026-11-23: Document the explicit Myanmar shaping fixture and its separation from layout’s fixed font database (#4314).
