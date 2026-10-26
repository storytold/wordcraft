# Installing spelling dictionaries

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-26 · **Change:** trivial (downloading a dictionary from File › Options › Proofing, pinned and checksummed; previously minor: ISO 8859-9 Turkish dictionaries are read, #3692; previously minor: Spanish: `es_ES`, `es_MX`, `es_419` and `es_UY` with their fallbacks, #3360, #3553; previously trivial: every Hunspell language shows (no dictionary), not only Polish and Portuguese, and Indonesian is listed, #3150; previously trivial: ISO 8859-1 dictionaries are read, #2824)

English spelling works out of the box. Polish, Russian, Portuguese, Czech, Italian, Croatian,
Turkish, Indonesian and Spanish spelling use a standard **Hunspell** dictionary (an `.aff` and a `.dic` file) that
WordCraft reads at runtime. These dictionaries are not bundled: they are large, and several are
GPL or LGPL licensed. Without one, the language isn't checked. The status bar then shows the
language with **(no dictionary)**, for example *Português (Brasil) (no dictionary)*.

If a dictionary is installed but can't be read (an unsupported `SET` encoding, a file over the
size limit, a parse error), the language isn't checked either, and the status bar shows
**(dictionary unreadable)** instead. Its tooltip gives the `.dic` file and the reason, and
**File › Options › Proofing** lists the same reason next to the file.

Install a dictionary once, restart WordCraft, and the language is checked as you type.

## Which files each language needs

WordCraft looks for the first of these file names (each name means a `<name>.aff` and
`<name>.dic` pair in the same folder):

| Language | File names | Common source |
|---|---|---|
| Polish | `pl_PL`, `pl-PL`, `pl` | SJP.PL |
| Russian | `ru_RU`, `ru-RU`, `ru` | LibreOffice `dict-ru` |
| Portuguese (Brazil and Portugal) | `pt_BR`, `pt-BR`, `pt` | VERO (LibreOffice `dict-pt-BR`) |
| Czech | `cs_CZ`, `cs-CZ`, `cs` | LibreOffice `dict-cs` |
| Italian | `it_IT`, `it_CH`, `it-IT`, `it` | LibreOffice `dict-it` |
| Croatian | `hr_HR`, `hr-HR`, `hr` | LibreOffice `dict-hr` |
| Turkish | `tr_TR`, `tr-TR`, `tr` | LibreOffice `dict-tr` |
| Indonesian | `id_ID`, `id-ID`, `id`, `in_ID` | LibreOffice `dict-id` |
| Spanish (Spain) | `es_ES`, `es-ES`, `es_ANY`, `es` | LibreOffice `dict-es` (RLA-ES) |
| Spanish (Mexico) | `es_MX`, `es-MX`, `es_419`, `es-419`, `es_ANY`, `es` | LibreOffice `dict-es` (RLA-ES) |
| Spanish (Latin America, and the other Spanish regions of the Americas) | `es_419`, `es-419`, `es_ANY`, `es`, `es_US`, `es_MX`, `es_AR`, `es-AR` | LibreOffice `dict-es` (RLA-ES) |
| Spanish (Uruguay) | `es_UY`, `es-UY`, `es_AR`, `es-AR`, `es_419`, `es-419`, `es_ANY`, `es` | LibreOffice `dict-es` (RLA-ES) |

Every Portuguese region uses the same `pt_BR` dictionary, so text marked *Português (Portugal)*
is checked with it too.

Spanish has a dictionary per variety: text marked *Español (España)* uses `es_ES`, *Español
(México)* `es_MX`, *Español (Uruguay)* `es_UY`, and *Español (Latinoamérica)* and the other
Spanish regions of the Americas (Argentina, Colombia, the United States…) the first Latin
American one found. LibreOffice's `dict-es` has them all. Uruguayan Spanish also accepts the
`vos` forms of the dictionary's verbs (`tenés`, `vení`, `decime`, `decímelo`).

The files may be UTF-8 or the encoding named on the `.aff`'s `SET` line (ISO 8859-1, ISO 8859-2,
ISO 8859-9, KOI8-R and Windows-1251 are read as well; the original VERO `pt_BR` is ISO 8859-1, older
Turkish `tr_TR` dictionaries ISO 8859-9).

## Where WordCraft looks

The first folder holding a matching pair wins:

1. The folder set in **File › Options › Proofing › Dictionaries folder**. This is the easiest
   way on every system, and it works for an app started from the Start menu, Finder or the Dock.
   The same pane lists the dictionary file each language found.
2. `dictionaries` in WordCraft's settings folder, where a download goes when no Dictionaries
   folder is set (see [Downloading one from WordCraft](#downloading-one-from-wordcraft)).
3. The folder in the `WORDCRAFT_DICTIONARIES` environment variable.
4. The system's usual places:

| System | Folders |
|---|---|
| Windows | `%LOCALAPPDATA%\WordCraft\dictionaries`, `%LOCALAPPDATA%\hunspell`, `%APPDATA%\WordCraft\dictionaries`, `%APPDATA%\hunspell`, LibreOffice's `share\extensions\dict-*` under `Program Files` (on any drive) |
| macOS | `~/Library/Spelling`, `/Library/Spelling`, `/opt/homebrew/share/hunspell`, `/usr/local/share/hunspell`, `/Applications/LibreOffice.app/Contents/Resources/extensions/dict-*` |
| Linux and BSD | `~/.local/share/hunspell`, `/usr/share/hunspell`, `/usr/share/myspell`, `/usr/share/myspell/dicts`, `/usr/local/share/hunspell`, `/usr/local/share/myspell`, LibreOffice's `share/extensions/dict-*` under `/usr/lib/libreoffice` or `/opt/libreoffice` |

The web version can't read these folders, so these languages aren't spell-checked there.

## Downloading one from WordCraft

For Polish, Portuguese, Italian, Russian and Czech, **File › Options › Proofing › Dictionaries** shows a **Download** button beside a language with no dictionary, or with one that can't be read. It fetches the pair from [LibreOffice's dictionary collection](https://github.com/LibreOffice/dictionaries) and saves it in the Dictionaries folder above, or, when none is set, in `dictionaries` in WordCraft's settings folder, which is always searched. Nothing is bundled: each dictionary keeps its own licence (GPL, LGPL or MPL) and stays a separate file that WordCraft only reads.

The download is pinned. It comes from one fixed commit of the collection, and each file must match the SHA-256 WordCraft records for it (`crates/proof/src/libreoffice.rs`) before it is parsed or saved. A changed, truncated or tampered file is refused and nothing is written. Croatian, Turkish and Indonesian have no button yet: their collection dictionaries use `FLAG long` or `FLAG num`, which WordCraft doesn't read yet.

## Example: Brazilian Portuguese (`pt_BR`)

VERO, the Brazilian Portuguese dictionary LibreOffice ships, is available from the
[LibreOffice dictionaries repository](https://github.com/LibreOffice/dictionaries/tree/master/pt_BR)
(LGPLv3 / MPL). Any of these ways works.

### If LibreOffice is installed

Nothing to do: WordCraft finds LibreOffice's `dict-pt-BR` extension on its own. Restart
WordCraft after installing LibreOffice.

### Windows

In PowerShell:

```powershell
$d = "$env:LOCALAPPDATA\WordCraft\dictionaries"
New-Item -ItemType Directory -Force $d | Out-Null
$base = "https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR"
Invoke-WebRequest "$base/pt_BR.aff" -OutFile "$d\pt_BR.aff" -UseBasicParsing
Invoke-WebRequest "$base/pt_BR.dic" -OutFile "$d\pt_BR.dic" -UseBasicParsing
```

### macOS

```sh
mkdir -p ~/Library/Spelling
cd ~/Library/Spelling
curl -fLO https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR/pt_BR.aff
curl -fLO https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR/pt_BR.dic
```

### Linux

From the distribution's packages:

```sh
sudo apt install hunspell-pt-br   # Debian, Ubuntu
```

Or, without root, into your own folder:

```sh
mkdir -p ~/.local/share/hunspell
cd ~/.local/share/hunspell
curl -fLO https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR/pt_BR.aff
curl -fLO https://raw.githubusercontent.com/LibreOffice/dictionaries/master/pt_BR/pt_BR.dic
```

Other languages work the same way: replace `pt_BR` with the language's folder and file names
from the LibreOffice repository (for example `it_IT/it_IT.aff`; the Spanish ones are all in
`es/`, for example `es/es_MX.aff`).

## Checking that it works

- Restart WordCraft. The status bar no longer says **(no dictionary)**, and a misspelled word
  such as `impramir` is underlined, with `imprimir` among its suggestions.
- **File › Options › Proofing** lists the file each language found.
- From the command line:

  ```sh
  wordcraft-cli run --cmd 'text.insert={"text":"Vou impramir o documento."}' --cmd select.all --cmd 'review.language={"lang":"pt-BR"}' --cmd review.issues --print
  ```

  The `review.issues` line lists `impramir` as a spelling issue with its suggestions.
- The log file (`%APPDATA%\WordCraft\logs\wordcraft.log` on Windows) says
  `Portuguese spelling: no Hunspell pt_BR dictionary installed` when none was found.

Never commit dictionary files to this repository: they stay on the user's system.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-26 | trivial | Downloading a dictionary from File › Options › Proofing: pinned to one commit of LibreOffice's collection, each file checked against its SHA-256 (#1698) |
| 2026-10-22 | minor | Spanish (#3360, #3553): the file names for Spain, Mexico, Latin America and Uruguay and their fallbacks |
| 2026-10-22 | trivial | Italian, Croatian, Turkish, Russian, Czech and Indonesian without a dictionary now show **(no dictionary)** (or **(dictionary unreadable)**) too, as this page already said; before, only Polish and Portuguese did. Indonesian is listed with its file names (#3150) |
| 2026-10-23 | trivial | ISO 8859-9 (`SET ISO8859-9`) dictionaries are read, so an older Turkish `tr_TR` loads without converting it to UTF-8; the Turkish dictionary is read through the same size-capped reader and `SET` decoding as the others (#3692) |
| 2026-10-18 | trivial | ISO 8859-1 (`SET ISO8859-1`) dictionaries are read, so the original VERO `pt_BR` works without converting it to UTF-8 (#2824) |
| 2026-10-18 | minor | An installed dictionary that can't be read shows **(dictionary unreadable)** in the status bar, with the file and reason in its tooltip and in File › Options › Proofing (#2830) |
| 2026-10-16 | major | First version: the file names per language, the search order and folders on Windows, macOS and Linux, installing `pt_BR` on each, and how to check it works |
