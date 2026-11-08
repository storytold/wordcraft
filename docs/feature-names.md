# Feature naming

WordCraft uses **Diagram**, **Diagram Design**, **Diagram Styles**, and **Decorative Text** for its own creation and editing features. Existing command IDs, public Rust type names and file-format mappings remain unchanged.

## Stable identities

The saved/control-channel contextual tab key `SmartArt Design` remains stable. Display-time localization maps that key to Diagram Design; dispatch and saved ribbon layouts do not depend on the new visible name. `insert.smartArt`, `smartArt.*`, `insert.wordArt`, `wordArt.*`, dialog IDs and icon IDs remain stable.

OOXML diagram/WordArt mappings, existing document-owned names and attribution are untouched. This is a naming change, not a legal determination.

## Translation dependency and readiness

WordCraft embeds shared catalogs from craft-i18n at `craft-i18n.rev`. The companion catalog PR must merge, then this pin must be advanced to its upstream commit before this app draft is marked ready. Until then, newly renamed labels can fall back to English. Do not merge the app change first.

The companion includes localized feature labels for every offered non-English language. Native-speaker review, the normal engine/UI/OOXML tests, visual ribbon/dialog checks and full CI remain required.
