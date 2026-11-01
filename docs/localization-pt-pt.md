# European Portuguese interface and proofing

> **Last reviewed:** 2026-10-24 · **Last updated:** 2026-11-01 · **Change:** trivial (the dictionary's source and licences; previously minor: pt-PT spelling from the installed `pt_PT` dictionary, Review › Language and the Region list; the interface catalog goes to storytold/craft-i18n as `locales/pt-pt/wordcraft.tsv`) · **Target:** WordCraft 0.10.0

## Proofing

Text tagged `pt-PT`, or one of the regions that write like Portugal (`pt-AO`, `pt-MZ`, `pt-CV`,
`pt-GW`, `pt-ST`, `pt-TL`, `pt-MO`, `pt-GQ`), is `ProofLang::PtPt`. Brazilian Portuguese, a bare
`pt` and any other region stay `ProofLang::Pt`. European text is spelled against the Hunspell
`pt_PT` dictionary installed on the system, found in the same places as the other dictionaries
([spelling-dictionaries.md](spelling-dictionaries.md)), LibreOffice's `dict-pt-PT` extension
included. The dictionary is not bundled and WordCraft doesn't download it (its README lists GPL 2,
LGPL 2.1 and MPL 1.1). Without a `pt_PT` dictionary,
European text is checked with the Brazilian one, as it was before the two spellings were told
apart; with neither, there are no spelling marks and the status bar says
*Portuguese (Portugal) (no dictionary)*.

With the dictionary of the Natura project (Departamento de Informática, Universidade do Minho,
<http://natura.di.uminho.pt/>, the one LibreOffice ships) the
affix engine reads all 42,642 stems and 1,290 rules: `facto`, `equipa`, `ecrã`, `ficheiro` and
`económico` are words, `econômico`, `ônibus` and `direcção` are flagged, and suggestions come from
the same dictionary (`economico` → `económico`, `tambem` → `também`, `coraçao` → `coração`).
Grammar rules and hyphenation patterns are shared with Brazilian Portuguese; the issues carry the
run's language (`ProofLang::PtPt`; `review.issues` reports `"lang": "pt"`, as it reports `"en"` for
every English). **Review › Language** offers **Portuguese (Portugal)**, File › Options › Proofing
lists the `pt_PT` file it found (or the `pt_BR` one it falls back to), and File › Options ›
Advanced › Region offers `pt-PT`.

## Interface

The interface catalogs live in [storytold/craft-i18n](https://github.com/storytold/craft-i18n).
The European Portuguese one goes there as `locales/pt-pt/wordcraft.tsv`, with its
`@id language.name` row, *Português (Portugal)*. Once `craft-i18n.rev` pins a commit that has it,
WordCraft lists `pt-pt` in `LANGUAGES` (with `pt-PT` as the language new documents are written
in) and resolves `pt-PT` and the regions above to it. Until then a `pt-PT` system locale keeps the
English interface, and `pt-BR` keeps the Brazilian one.

The catalog has all 2,061 entries of the Brazilian catalog as it was in WordCraft before the move
(the same keys). It was adapted from WordCraft's own `pt-br.tsv` and the meaning of the English
labels; no proprietary translation resources were consulted. 574 entries differ from the
Brazilian ones. Strings added later fall back to English until they are translated.

### Terminology

| Source | European Portuguese | Brazilian Portuguese |
|---|---|---|
| File / Save / Delete | Ficheiro / Guardar / Eliminar | Arquivo / Salvar / Excluir |
| Font / Character | Tipo de letra / Carácter | Fonte / Caractere |
| Undo / Reset | Anular / Repor | Desfazer / Redefinir |
| Cut / Crop | Cortar / Recortar | Recortar / Cortar |
| Section / Record | Secção / Registo | Seção / Registro |
| Password / User Name | Palavra-passe / Nome de utilizador | Senha / Nome de usuário |
| Indent / Border | Avanço / Limite | Recuo / Borda |
| Bullets / Bookmark | Marcas / Marcador | Marcadores / Indicador |
| Table of Contents / Index | Índice / Índice remissivo | Sumário / Índice |
| Track Changes / Tracked changes | Registar alterações / Alterações registadas | Controlar alterações / Alterações controladas |
| Mail Merge / Merge Cells | Impressão em série / Unir células | Mala direta / Mesclar células |
| Link / Layout / View | Ligação / Esquema / Ver | Link / Layout / Exibir |
| Ribbon / tab | Friso / separador | Faixa de opções / guia |
| Settings / Default | Definições / Predefinição | Configurações / Padrão |
| Strikethrough / Subscript / Superscript | Rasurado / Inferior à linha / Superior à linha | Tachado / Subscrito / Sobrescrito |
| Landscape / Portrait | Horizontal / Vertical | Paisagem / Retrato |
| Synonyms / Polish / Czech | Sinónimos / Polaco / Checo | Sinônimos / Polonês / Tcheco |

Grammar follows European usage: no `você` (`Ao escrever`, not `Enquanto você digita`), the article
before possessives (`as suas alterações`), enclitic pronouns (`substitui-o`, `ignoram-nos`),
`a` + infinitive for the progressive (`A aguardar o Zotero…`), and `num`/`numa`. Spelling follows
the 1990 agreement as applied in Portugal (`secção`, `facto`, `carácter`, `económico`, `hífenes`).
Product and format names stay as they are.

## Verification and remaining scope

`cargo test -p wordcraft-proof` covers the region split (`pt-PT`, `pt_AO`, `pt-Latn-PT`, `pt-BR`,
bare `pt`, private-use `pt-x-pt`), the two dictionaries side by side with synthetic test
dictionaries, the file names each spelling looks for and the fallback to `pt_BR`, suggestions per
spelling, the proofing options, grammar issues carrying `ProofLang::PtPt`, list items, and
hyphenation.
`cargo test -p wordcraft-ui-egui` covers the Review › Language entry and its check mark, and the
names of the other regions (`pt-MZ` → *Portuguese (Mozambique)*, proofed as European).

Not covered yet: hyphenated prefixed forms and some clitic combinations the dictionary doesn't
derive (`fê-lo`), and a review of the catalog by a second native speaker, which is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-01 | trivial | The `pt_PT` dictionary's source and licences, from its README; not bundled, not downloaded |
| 2026-10-24 | minor | Rebased on the craft-i18n cutover: the catalog goes to storytold/craft-i18n; pt-PT proofing next to the Spanish, Italian, Croatian and Turkish ones |
| 2026-10-12 | minor | European Portuguese catalog (2,061 entries), terminology, pt-PT proofing against the installed `pt_PT` dictionary, and regression checks |
