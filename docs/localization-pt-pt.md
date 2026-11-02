# European Portuguese interface and proofing

> **Last reviewed:** 2026-11-02 · **Last updated:** 2026-11-02 · **Change:** minor (proofing moved to craft-proof in storytold/craft-libs, #5202, which reads `pt-PT` text against its own `pt_PT` dictionary; this page now says what WordCraft does at its pin and what craft-proof does; previously trivial: the dictionary's source and licences; previously minor: pt-PT spelling from the installed `pt_PT` dictionary, Review › Language and the Region list; the interface catalog goes to storytold/craft-i18n as `locales/pt-pt/wordcraft.tsv`) · **Target:** WordCraft 0.10.0

## Language menu and region

**Review › Language** offers **Portuguese (Portugal)** (`pt-PT`) next to Portuguese (Brazil). Its
check mark and the status bar's name follow the region the tag names, however it is written
(`pt_PT.UTF-8`, `pt_PT@euro`, `por-PT`, `pt-Latn-PT`); a Portuguese tag with no region is
Brazil's, and another region shows by name (`pt-MZ` → *Portuguese (Mozambique)*).
**File › Options › Advanced › Region** offers `pt-PT`.

## Proofing

Proofing is craft-proof, in [storytold/craft-libs](https://github.com/storytold/craft-libs)
`crates/proof` (#5202), so the spelling of `pt-PT` text is decided there:

- **At the craft-proof commit WordCraft pins now (`48ed6f8`)**, every Portuguese tag is checked
  with the Hunspell `pt_BR` dictionary ([spelling-dictionaries.md](spelling-dictionaries.md)), so
  `facto`, `equipa` and `económico` are flagged in European text.
- **craft-libs main** reads `pt-PT` text against its own dictionary: `pt_PT` (or `pt-PT`, `pt`)
  in the dictionaries folder, a `pt-pt` Language pack, then the system's `pt_PT`, with the
  Portuguese grammar rules, messages and hyphenation patterns. WordCraft gets this when its pin
  moves.

The `pt_PT` dictionary LibreOffice ships (`dict-pt-PT`) is the Natura project's (Departamento de
Informática, Universidade do Minho, <http://natura.di.uminho.pt/>); its README lists GPL 2,
LGPL 2.1 and MPL 1.1. WordCraft doesn't bundle or download it.

Still to do in craft-proof: the regions that write like Portugal (`pt-AO`, `pt-MZ`, `pt-CV`,
`pt-GW`, `pt-ST`, `pt-TL`, `pt-MO`) are read as Brazilian there, and some hyphenated prefixed
forms and clitic combinations the dictionary doesn't derive (`fê-lo`) are flagged.

## Interface

The interface catalogs live in [storytold/craft-i18n](https://github.com/storytold/craft-i18n).
The European Portuguese one goes there as `locales/pt-pt/wordcraft.tsv`, with its
`@id language.name` row, *Português (Portugal)*. Once `craft-i18n.rev` pins a commit that has it,
WordCraft lists `pt-pt` in `LANGUAGES` (with `pt-PT` as the language new documents are written
in) and resolves `pt-PT` to it. Until then a `pt-PT` system locale keeps the English interface,
and `pt-BR` keeps the Brazilian one.

The catalog started from all 2,061 entries of the Brazilian catalog as it was in WordCraft before
the move (the same keys), adapted from WordCraft's own `pt-br.tsv` and the meaning of the English
labels; 574 of them differ from the Brazilian ones. The strings WordCraft gained since were
written from the meaning of the English, for 3,050 entries in all. No proprietary translation
resources were consulted.

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

`cargo test -p wordcraft-ui-egui` covers the Review › Language entry, its name and check mark for
the ways a `pt-PT` tag is written, and the names of the other regions.

Not covered yet: a review of the catalog by a second native speaker, which is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-02 | minor | Proofing moved to craft-proof (#5202), which reads `pt-PT` against its own `pt_PT` dictionary on craft-libs main; WordCraft's own pt-PT proofing code is gone, and this page says what each does. The catalog has 3,050 entries |
| 2026-11-01 | trivial | The `pt_PT` dictionary's source and licences, from its README; not bundled, not downloaded |
| 2026-10-24 | minor | Rebased on the craft-i18n cutover: the catalog goes to storytold/craft-i18n; pt-PT proofing next to the Spanish, Italian, Croatian and Turkish ones |
| 2026-10-12 | minor | European Portuguese catalog (2,061 entries), terminology, pt-PT proofing against the installed `pt_PT` dictionary, and regression checks |
