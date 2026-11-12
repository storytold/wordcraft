# Layout test fonts

Layout unit tests use only the 13 font files already pinned under `assets/fonts/`: Source Sans 3,
Source Serif 4, Inter and JetBrains Mono, with their bundled styles. Family listings also include
legacy style names such as Inter Medium. They do not load installed fonts, user font preferences,
or the optional `CRAFT_FONTS_DIR` input. No download or extra font checkout is required.

Every layout unit test starts with `crate::test_fonts();`, before constructing its fixture or
resolving a font. This initializes the process-wide database once, including when a single test is
run by name. The `wordcraft-fonts/test-support` development feature exposes the initializer;
enabling the feature alone has no effect. Initialization fails if an application database already
exists, rather than replacing a database whose faces may be in the shaping or math caches.
Other test binaries and application builds keep their normal font discovery behavior.

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
