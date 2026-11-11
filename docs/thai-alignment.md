# Thai paragraph alignment

Status: 2026-11-06. This change adds a stored Thai Distributed alignment value,
command/UI support and distinct DOCX round trips. HTML, RTF and ODT export it as Justify.
Wrapped Thai Distributed lines expand spaces first (0.5em cap), then safe
Thai group gaps (0.15em cap), using the smaller adjacent font size. Final and explicit-break
lines retain natural spacing. Ordinary Justify retains its existing spacing and manual-break
behavior. Drawing, caret, selection and PDF use the same geometry. The caps and the automatic
command and dialog choices have not been compared with Word.

This branch retains the base wrapping algorithm. Dictionary segmentation and safe emergency
wrapping are reviewed independently in PR #1713. No ICU dependency is added here.
