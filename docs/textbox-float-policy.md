# Floating rectangles inside fitted textboxes

> **Last updated:** 2026-10-21 · **Change:** trivial (bounded static outer fit-cache reuse, #2533).

For direct floating, unlinked autoFit textboxes with horizontal, top-aligned text, direct non-text rectangles using Front, Behind or Square wrapping do not paint or enlarge the fitted frame through their own bounds. Square wrapping still displaces text before fitting. Source objects, anchor characters and hit metadata remain available for editing and saving.

The rule matches source story, paragraph path and byte offset. Both shape paint and hit metadata must identify the eligible source rectangle. Equal rectangles do not establish ownership. Paragraph markers must match the object count.

The bounded class excludes inline parents, linked or turned boxes, center/bottom alignment, groups, nested textbox content, table-contained children, other shape kinds, outlined/effected/turned children and other wrapping modes. Children use column/paragraph anchoring without alignment overrides. Existing behavior remains for those inputs. Saved-frame fallback, shared alignment extent, measurement budget and counter rollback stay unchanged.

Six original private native controls cover Plain, Front/Behind inside/crossing rectangles and Square wrapping. Plain and Front/Behind frames measure 17 pt; Square measures 57 pt after text displacement. The qualified matched-reader comparison retains these frame and paint distinctions. Eligible rectangles do not paint, including those inside the parent frame.

The published #2453 reader is identical on both fitting bases. TallInline retains its inline image, source owner, hit metadata and line ownership. Parent and candidate both measure 104.380859 pt, versus 103.1 pt in the native export. The inherited height delta is +1.280857 pt; the image origin is -0.5 pt on each axis, with unchanged image dimensions. This does not establish exact native TallInline geometry or raster parity. The six native cases establish only the bounded fitting and own-paint policy above.

## Repeated fitting probes

Completed outer measurements of static, unlinked textbox graphs can reuse raw content extent, including an exhausted result that keeps the current caller's saved height. Reuse requires the same inner width, direct-parent classification, depth, grid and unchanged persistent inline-fitted cache prefix. Caller outline and margins are applied after lookup. The first measurement retains existing budget and rollback behavior.

Linked graphs, dynamic content and nested measurements retain fresh probes. Linked flow maps can be replaced by header/footer probes, so their journal lengths do not establish a stable cache context. This optimization changes no fitting ownership, source markers, hit/line metadata or own-paint policy. It establishes no universal once-per-pass, cache-memory or new native-parity claim. The inherited TallInline limits above remain.

## Revision history

| Date | Change |
|---|---|
| 2026-10-21 | Bounded static outer fit-cache reuse; linked, dynamic and nested graphs retain fresh probes. |
