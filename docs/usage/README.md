# WordCraft usage guides and translations

The English guide is the source text for this handbook. German is the first example translation;
other languages are welcome. Documentation language is separate from the app's interface language.

| Language | Guide | Coverage |
|---|---|---|
| English | [User guide](usage.md) | Initial workflows and shortcut reference; remaining coverage is listed in the guide |
| Deutsch | [Bedienungsanleitung](usage-de.md) | Erste Beispielübersetzung mit demselben Umfang; offene Themen stehen in der Anleitung |

## Add another language

1. Start from [usage.md](usage.md) and create `usage-<language-code>.md` in this folder.
   For example, French would use `usage-fr.md` and Brazilian Portuguese `usage-pt-BR.md`.
   These are naming examples, not translations that already exist.
2. Translate the guide's explanations and contents links. Keep matching topic coverage, examples,
   shortcut values, contexts and limitations. Retain the exact English UI labels where useful
   for locating commands, with an explanation in the target language.
3. Link to existing detailed topic pages instead of duplicating them. Say when those linked
   pages are in another language. Check relative links and the translated heading anchors.
4. Include a link back to this index and to the English guide. Add the completed guide to the
   table above; the project README links here so every language remains discoverable.
5. Review the translation for meaning and verify shortcut/platform claims against the implementation.
   Record the source revision/date checked in the guide. Do not imply a platform was tested
   merely because its keys were translated.

Keep the English guide and affected existing translations current when behavior changes, as
required by [AGENTS.md](../../AGENTS.md#user-documentation). Add languages when their guide is
available; do not create empty files or list planned translations as complete.
