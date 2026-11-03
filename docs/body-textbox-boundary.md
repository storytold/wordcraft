# Wrapping body textboxes at page boundaries

> **Last updated:** 2026-10-25 · **Change:** trivial (preserve explicit modern drops across body margins, #3169; retain bounded textbox placement, #2731).

In Print view, unturned wrapping textboxes anchored in top-level body paragraphs use their fitted frame before placement. Compatibility modes below 15 keep fitting frames within the physical page. In modern modes, fitting Paragraph/Line anchors stay between the text area's top and bottom margins. Modern Page/Margin anchors retain absolute placement and may overflow.

Explicit modern body-textbox drops across either body margin retain a vertical Page anchor, using the fitted frame to decide. In-body drops return to Paragraph anchoring. Body margins come from the anchor paragraph's owning section, including continuous sections sharing a physical page. This preserves the literal drop coordinates required by #3169 without relaxing imported Paragraph/Line bounds. Pictures and shapes without a valid TextBox story keep their existing placement; a story reference alone does not qualify for textbox bounds. Legacy physical-page bounds remain. These are model and engine controls, with no new native or corpus qualification.

A modern Paragraph/Line frame taller than the text area starts after its anchor line, on a fresh column when needed. It keeps its full frame height. Oversized wrapping and paragraph continuation retain the split policy; its cache key stays separate from ordinary wrapping. Non-textbox floats, turned boxes, inline and Front/Behind objects retain their existing behavior.

If a corrected frame reaches earlier body text, bounded replay supplies that wrap area before the text is laid out again. Replay is capped at four retries. This does not establish convergence or a memory bound. Public paragraph callers retain their existing API and ordinary long-word behavior. Removing alignment keeps page-fixed drawings page-fixed.

Paragraph continuations also receive later-anchor wrap areas found for their new page. Each area retains its page coordinates and Split/Wait policy. A private replay boundary keeps estimated row probes from changing the placed prefix; that boundary is part of the paragraph cache identity. The relay still requires the same resume byte and top. Some full-width waits and contours change that boundary: the guard retains fallback, and those overlaps remain outside this repair. Original Paragraph/Line model controls cover cross-page text/frame separation, conserved ranges and the unchanged prefix.

Original model/import controls and private native exports cover representative legacy/modern anchoring, fitted frames, oversized placement and backward wrapping. Three known omitted-spacing frame offsets remain about 0.1221 pt. The inherited TallInline height difference is about +1.280857 pt, with a -0.5 pt image-origin offset on each axis. These are explicit limits, not exact native geometry or raster parity.

The reader, source markers, hit/line ownership, descendant own-paint policy and static outer fit-cache contract remain those of [textbox-float-policy.md](textbox-float-policy.md). No broader nested-descendant rule is added.

## Revision history

| Date | Change |
|---|---|
| 2026-10-25 | Explicit modern drops across body margins retain vertical Page anchoring; textbox bounds require a TextBox part and use the anchor paragraph's owning section margins. Imported bounds and historical native limits remain. |
| 2026-10-21 | Continued body paragraphs receive later-anchor wrap areas on their new page (#2731). A private replay boundary preserves the placed prefix; relays that change the resume byte or top retain fallback, including some contour and full-width cases. |
| 2026-10-21 | Bounded body textbox placement and backward wrap replay; inherited native geometry limits retained. |
