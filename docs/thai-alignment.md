# Thai paragraph alignment

Status: 2026-11-06. This change adds a stored Thai Distributed alignment value,
command/UI support and distinct DOCX round trips. HTML, RTF and ODT export it as Justify.
Wrapped Thai Justify/Thai Distributed lines expand spaces first (0.5em cap), then safe
Thai group gaps (0.15em cap), using the smaller adjacent font size. Final and explicit-break
lines retain natural spacing. Drawing, caret, selection and PDF use the same geometry.

This branch retains the base wrapping algorithm. Dictionary segmentation and safe emergency
wrapping are reviewed independently in PR #1713. No ICU dependency is added here.
