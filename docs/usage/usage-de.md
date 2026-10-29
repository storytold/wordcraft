# WordCraft – Bedienungsanleitung

[English](usage.md) · [Alle Sprachen / Übersetzen](README.md) · [Projekt-README](../../README.md)

Englisch ist die Ausgangsfassung; Deutsch dient als erste Beispielübersetzung. Weitere Sprachen
sind nach demselben Aufbau willkommen (siehe Sprachindex).

Diese Anleitung bündelt alltägliche Abläufe und Tastenkürzel. Englische Befehlsnamen bleiben zur
Orientierung im Menüband und in der Befehlssuche erhalten; die Oberfläche kann sie übersetzen.
Abgeglichen mit Quellstand [2cf1d6c](https://github.com/storytold/wordcraft/tree/2cf1d6c7ae94723a45bd6e7e8c884fe375148750)
am 27.10.2026. Eine heruntergeladene Version kann davon abweichen.

## Inhalt

- [Starten, speichern und exportieren](#starten-speichern-und-exportieren)
- [Schreiben, auswählen, kopieren und einfügen](#schreiben-auswählen-kopieren-und-einfügen)
- [Dokument formatieren und strukturieren](#dokument-formatieren-und-strukturieren)
- [Überprüfen, Verweise und Seriendruck](#überprüfen-verweise-und-seriendruck)
- [Einstellungen und eigene Tastenkürzel](#einstellungen-und-eigene-tastenkürzel)
- [Tastenkürzelreferenz](#tastenkürzelreferenz)
- [Kontextabhängige Bedienung](#kontextabhängige-bedienung)
- [Weitere Hilfe und Abdeckung](#weitere-hilfe-und-abdeckung)

## Starten, speichern und exportieren

1. Unter **File › New** ein leeres Dokument oder eine Vorlage wählen; mit **File › Open** eine
   vorhandene Datei öffnen.
2. Im Dokument schreiben. Unter **File › Save As** Namen, Speicherort und Format festlegen;
   weitere Änderungen mit **Save** speichern. Um die Eingabedatei zu behalten, einen neuen Namen wählen.
3. **File › Export › Export PDF** erstellt eine PDF-Kopie. **File › Print** öffnet die
   Druckeinstellungen. Mit **Print Markup** festlegen, ob Kommentare und nachverfolgte Änderungen
   in der Ausgabe erscheinen.

WordCraft liest und schreibt unter anderem DOCX, ODT, RTF, HTML, Markdown und Text;
das ältere DOC wird importiert. Eine Konvertierung kann Inhalt oder Layout verändern: Vor einem
Formatwechsel die [Formatreferenz](../file-format-parity.md) beachten und die gespeicherte bzw.
exportierte Datei prüfen. Installation und Build bleiben im [README](../../README.md) dokumentiert.
Bei unterbrochener Arbeit hilft die [Wiederherstellungsanleitung](../recovery.md).

## Schreiben, auswählen, kopieren und einfügen

Ein Klick setzt die Einfügemarke, Ziehen markiert Text. **Shift** beim Bewegen der Einfügemarke
erweitert die Auswahl. **Enter** beginnt einen Absatz, **Shift+Enter** fügt einen Zeilenumbruch ein.
In Listen setzt Enter die Liste fort und Tab ändert die Ebene; in Tabellen bewegt Tab zwischen
Zellen. Die [Dokumentation zum Schreibverhalten](../typing-parity.md) beschreibt Details und Ausnahmen.

Text markieren und **Home › Clipboard › Copy** oder **Cut** wählen, dann am Ziel die Einfügemarke
setzen und **Paste** ausführen. Normales Einfügen nutzt vorhandene Zwischenablageformatierung;
das Ergebnis hängt von den Formaten der Quellanwendung ab. **Paste: Keep Text Only** fügt reinen
Text ein, **Paste Special** bietet die verfügbaren Zwischenablageformate an.

Die Standardbelegung unterscheidet **Textinhalt** und **Formatierung**:

| Aktion | Windows / Linux / BSD | macOS |
|---|---|---|
| Inhalt kopieren | Ctrl+C / Ctrl+Insert | Cmd+C / Cmd+Insert |
| Inhalt einfügen | Ctrl+V / Shift+Insert | Cmd+V / Shift+Insert |
| Nur Text einfügen | Ctrl+Shift+Alt+V | Cmd+Shift+Alt+V |
| Formatierung kopieren | Ctrl+Shift+C | Cmd+Shift+C |
| Formatierung übertragen | Ctrl+Shift+V | Cmd+Shift+V |
| Inhalte einfügen (Paste Special) | Ctrl+Alt+V | Cmd+Alt+V |

**Copy Formatting / Paste Formatting** übertragen die Formatierung der Quellauswahl auf das Ziel;
sie kopieren oder fügen keinen reinen Text ein. Im geprüften Befehlsverzeichnis gibt es weder ein
Standardkürzel für unformatiertes Kopieren noch für Ctrl+Alt+Insert oder Shift+Alt+Insert.

Unter **Home › Editing › Find** suchen, mit **Find and Replace** Treffer ersetzen und mit **Go To**
navigieren. **Undo** macht eine Bearbeitung rückgängig, **Redo** stellt sie wieder her.
**Repeat** führt die letzte wiederholbare Aktion erneut aus und ist eine andere Funktion als Redo.

## Dokument formatieren und strukturieren

Vorher Text markieren oder die Einfügemarke in den gewünschten Absatz setzen.

| Aufgabe | Wo und wie | Ergebnis / Kontext |
|---|---|---|
| Zeichen gestalten | **Home › Font**: Schriftart, Größe, Hervorhebung, Farbe wählen | Formatiert die Auswahl oder anschließend eingegebenen Text |
| Absätze gestalten | **Home › Paragraph**: Ausrichtung, Abstände, Einzüge oder Liste wählen | Ändert die ausgewählten Absätze |
| Einheitliche Überschriften | Unter **Home › Styles** Normal oder eine Heading-Vorlage anwenden | Überschriften strukturieren das Dokument; Vorlagen im Styles-Bereich verwalten |
| Seiten einrichten | Unter **Layout** Ränder, Ausrichtung, Papiergröße, Spalten und Umbrüche einstellen | Ändert Seiten-/Abschnittslayout; Einschränkungen stehen in der [Layoutreferenz](../layout-parity.md) |
| Tabelle einfügen | **Insert › Table**, Abmessungen wählen und Zellen bearbeiten | Tabellenbefehle wirken auf die aktuelle Tabelle/Zellauswahl; Tab wechselt Zellen |
| Bilder, Formen oder Diagramme | Passenden **Insert**-Befehl wählen und das eingefügte Objekt auswählen | Kontextsteuerung für Größe, Position und Textumbruch verwenden |
| Link oder Textmarke | **Insert › Links**: Text wählen, Link oder Bookmark öffnen und Ziel/Namen eingeben | Erstellt einen navigierbaren Verweis |
| Kopf-/Fußzeilen und Felder | Unter **Insert** die Kopf-/Fußzeilenbefehle bzw. **Text › Quick Parts** verwenden | Felder lassen sich aktualisieren; Kopf-/Fußzeilen gehören zu Abschnitten |
| Ansicht wechseln | Unter **View** Print Layout, Draft, Outline, Read Mode oder geteilte Ansicht wählen | Ändert die Arbeitsansicht, nicht die gespeicherte Seitengröße |

## Überprüfen, Verweise und Seriendruck

- **Review:** **Track Changes** vor den zu protokollierenden Bearbeitungen einschalten. Für einen
  Kommentar Text markieren und **New Comment** wählen. Mit den Prüfwerkzeugen Änderungen annehmen
  oder ablehnen und zwischen Kommentaren wechseln. Ausblenden von Markierungen nimmt Änderungen
  nicht an. **Spelling & Grammar** prüft die Sprache; die [Wörterbuchanleitung](../spelling-dictionaries.md)
  erklärt Installation und Auswahl.
- **References:** Vor dem Inhaltsverzeichnis Überschriftenvorlagen anwenden. **Insert Footnote**
  oder **Insert Endnote** an der gewünschten Verweisposition wählen. Felder nach Dokumentänderungen
  aktualisieren. Zitate, Literaturverzeichnis, Beschriftungen und Indexeinträge liegen unter
  **References**; für [Zotero](../zotero.md) gibt es eine eigene Einrichtungs- und Bedienungsanleitung.
- **Mailings:** Empfänger auswählen, Seriendruckfelder ins Dokument einfügen und **Preview Results**
  prüfen. Danach **Finish & Merge** oder **Print Documents** verwenden. Datensätze in der Vorschau
  prüfen, bevor die Ausgabe erstellt wird. Eine konfigurierte Empfängerquelle ist erforderlich.

## Einstellungen und eigene Tastenkürzel

Anwendungseinstellungen liegen unter **File › Options**. Die Tastenkürzel stehen unter
**Tools › Customize Keyboard** oder **File › Options › Keyboard shortcuts**:

1. Kategorie und Befehl auswählen oder den Dialogfilter verwenden.
2. Aktuelle Belegungen prüfen. **New shortcut** fokussieren und die gewünschte Kombination drücken.
3. Angezeigte Zuordnung prüfen und mit **Assign** zuweisen. Eine vorhandene Belegung mit **Remove**
   entfernen. **Reset All** stellt nach einer Bestätigung die Standardbelegung wieder her.

Die Belegungen werden in den Einstellungen gespeichert. Der Dialog akzeptiert Kombinationen mit
Modifikatortasten oder Funktionstasten statt unmodifizierter Buchstabentasten. Eigene Zuordnungen
und entfernte Belegungen können von den Tabellen abweichen.
Unter Windows/Linux/BSD klappt **Ctrl+F1** das Menüband ein/aus, sofern kein anderer Befehl zugewiesen
wurde; unter macOS gilt **Cmd+Alt+R**. **Alt+Q** öffnet auf Nicht-macOS-Builds die Befehlssuche.

## Tastenkürzelreferenz

Aufgeführt sind die **163 Befehle mit Standardkürzeln** des geprüften Engine-Verzeichnisses,
nach Bereichen sortiert; anschließend folgen Oberflächen- und Kontextsteuerungen. Befehle ohne
Standardkürzel sind, soweit unterstützt, über Oberfläche oder Befehlssuche erreichbar. Dies ist
ein Quellcodeabgleich, keine Bestätigung eines praktischen Tests jeder Kombination auf jeder Plattform.

- **Ctrl** heißt auf deutschen Tastaturen **Strg**, **Shift** bedeutet **Umschalt**,
  **Insert** bedeutet **Einfg**, **Delete** bedeutet **Entf**, **Home/End** bedeutet **Pos1/Ende**,
  **PageUp/PageDown** bedeutet **Bild auf/ab**, **Space** bedeutet **Leertaste**.
  **Cmd** ist Command (⌘), **Alt** auf macOS Option (⌥). Der Schrägstrich trennt Alternativen;
  **—** bedeutet keine Standardbelegung auf dieser Plattform.
- Die Plattformspalten gelten für native Builds. Der Web-Build nutzt die Nicht-macOS-Standardbelegungen
  des Verzeichnisses; seine primäre Modifikatortaste folgt der Browserplattform. Browser oder
  Betriebssystem können Kürzel vorher abfangen. Funktionstasten benötigen je nach Tastatur **Fn**.
- Für Bearbeitung und Navigation ins Dokument klicken. Ein fokussiertes Textfeld oder ein Dialog
  verarbeitet seine eigenen Eingaben. Gliederungs-, Feld-, Tabellen-, Prüf- und Seriendruckbefehle
  benötigen ihren jeweiligen Kontext. Ein Kürzel aktiviert keinen deaktivierten Befehl.
- Zeichenbezeichnungen entsprechen den registrierten Tasten; Satzzeichen liegen je nach Tastaturlayout
  an anderen Stellen. Die englischen Befehlsnamen werden um deutsche Erläuterungen ergänzt.

### Dateien

Quelle: [file](../../crates/engine/src/cmd/file.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| New — Neues Dokument | `Ctrl+N` | `Cmd+N` |
| Open — Öffnen | `Ctrl+O / Ctrl+F12 / Ctrl+Alt+F2` | `Cmd+O / Cmd+F12 / Cmd+Alt+F2` |
| Save — Speichern | `Ctrl+S / Shift+F12 / Alt+Shift+F2` | `Cmd+S / Shift+F12 / Alt+Shift+F2` |
| Save As — Speichern unter | `F12 / Alt+F2` | `F12 / Cmd+Shift+S` |
| Print — Drucken | `Ctrl+P / Ctrl+Shift+F12 / Ctrl+F2 / Ctrl+Alt+I` | `Cmd+P / Cmd+Shift+F12 / Cmd+F2 / Cmd+Alt+I` |
| Close — Schließen | `Ctrl+W / Ctrl+F4` | `Cmd+W / Cmd+F4` |

### Schreiben und Sonderzeichen

Quelle: [text](../../crates/engine/src/cmd/text.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| New Paragraph — Neuer Absatz | `Enter` | `Enter` |
| Line Break — Zeilenumbruch | `Shift+Enter` | `Shift+Enter` |
| Page Break — Seitenumbruch | `Ctrl+Enter` | `Cmd+Enter` |
| Column Break — Spaltenumbruch | `Ctrl+Shift+Enter` | `Cmd+Shift+Enter` |
| Tab — Tabulator bzw. nächster Listeneinzug/Tabellenzelle | `Tab` | `Tab` |
| Shift+Tab — Einzug zurück bzw. vorherige Tabellenzelle | `Shift+Tab` | `Shift+Tab` |
| Tab Character — Tabulatorzeichen einfügen | `Ctrl+Tab` | `Alt+Tab` |
| Backspace — Rückwärts löschen | `Backspace` | `Backspace` |
| Delete — Vorwärts löschen | `Delete` | `Delete` |
| Delete Previous Word — Vorheriges Wort löschen | `Ctrl+Backspace / Alt+Backspace` | `Cmd+Backspace / Alt+Backspace` |
| Delete Next Word — Nächstes Wort löschen | `Ctrl+Delete / Alt+Delete` | `Cmd+Delete / Alt+Delete` |
| Nonbreaking Space — Geschütztes Leerzeichen | `Ctrl+Shift+Space` | `Cmd+Shift+Space` |
| Nonbreaking Hyphen — Geschützter Bindestrich | `Ctrl+Shift+-` | `Cmd+Shift+-` |
| Optional Hyphen — Bedingter Trennstrich | `Ctrl+-` | `Cmd+-` |
| Toggle Unicode Character — Unicode-Code und Zeichen umwandeln | `Alt+X` | — |
| Em Dash — Geviertstrich | `Ctrl+Alt+-` | `Cmd+Alt+-` |
| Copyright Sign — Copyright-Zeichen | `Ctrl+Alt+C` | — |
| Registered Sign — Zeichen für eingetragene Marke | `Ctrl+Alt+R` | — |
| Trademark Sign — Markenzeichen | `Ctrl+Alt+T` | — |
| Ellipsis — Auslassungszeichen | `Ctrl+Alt+.` | — |

### Navigation und Auswahl

Quelle: [caret](../../crates/engine/src/cmd/caret.rs), [column](../../crates/engine/src/cmd/column.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Column Selection — Spaltenauswahl | `Ctrl+Shift+F8` | `Cmd+Shift+F8` |
| Left — Nach links | `Left` | `Left` |
| Right — Nach rechts | `Right` | `Right` |
| Up — Nach oben | `Up` | `Up` |
| Down — Nach unten | `Down` | `Down` |
| Word Left — Wort nach links | `Ctrl+Left / Alt+Left` | `Alt+Left` |
| Word Right — Wort nach rechts | `Ctrl+Right / Alt+Right` | `Alt+Right` |
| Line Start — Zeilenanfang | `Home` | `Home / Cmd+Left` |
| Line End — Zeilenende | `End` | `End / Cmd+Right` |
| Paragraph Up — Absatz nach oben | `Ctrl+Up / Alt+Up` | `Cmd+Up / Alt+Up` |
| Paragraph Down — Absatz nach unten | `Ctrl+Down / Alt+Down` | `Cmd+Down / Alt+Down` |
| Document Start — Dokumentanfang | `Ctrl+Home` | `Cmd+Home` |
| Document End — Dokumentende | `Ctrl+End` | `Cmd+End` |
| Page Up — Seite nach oben | `PageUp` | `PageUp` |
| Page Down — Seite nach unten | `PageDown` | `PageDown` |
| Select All — Alles auswählen | `Ctrl+A` | `Cmd+A` |
| Collapse Selection — Auswahl aufheben | `Escape` | `Escape` |

### Bearbeitung und Zwischenablage

Quelle: [edit](../../crates/engine/src/cmd/edit.rs), [paste](../../crates/engine/src/cmd/paste.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Undo — Rückgängig | `Ctrl+Z` | `Cmd+Z` |
| Redo — Wiederherstellen | `Ctrl+Y / Ctrl+Shift+Z` | `Cmd+Y / Cmd+Shift+Z` |
| Cut — Ausschneiden | `Ctrl+X` | `Cmd+X` |
| Copy — Kopieren | `Ctrl+C / Ctrl+Insert` | `Cmd+C / Cmd+Insert` |
| Spike — Auswahl ausschneiden und in der Sammlung ablegen | `Ctrl+F3` | `Cmd+F3` |
| Insert Spike — Gesammelte Ausschnitte einfügen | `Ctrl+Shift+F3` | `Cmd+Shift+F3` |
| Paste — Einfügen | `Ctrl+V / Shift+Insert` | `Cmd+V / Shift+Insert` |
| Paste: Keep Text Only — Nur Text einfügen | `Ctrl+Shift+Alt+V` | `Cmd+Shift+Alt+V` |
| Find — Suchen | `Ctrl+F` | `Cmd+F` |
| Find Next — Nächster Treffer | `Ctrl+G / F3 / Shift+F4 / Ctrl+PageDown` | `Cmd+G / F3 / Shift+F4 / Cmd+PageDown` |
| Find Previous — Vorheriger Treffer | `Ctrl+PageUp` | `Cmd+Shift+G / Cmd+PageUp` |
| Find and Replace — Suchen und Ersetzen | `Ctrl+H` | `Cmd+H / Ctrl+H` |
| Go To — Gehe zu | `Ctrl+Alt+G / F5` | `Cmd+Alt+G / F5` |
| Go Back — Zur letzten Bearbeitungsstelle zurück | `Shift+F5 / Ctrl+Alt+Z` | `Shift+F5 / Cmd+Alt+Z` |
| Copy Formatting — Formatierung kopieren | `Ctrl+Shift+C` | `Cmd+Shift+C` |
| Paste Formatting — Formatierung übertragen | `Ctrl+Shift+V` | `Cmd+Shift+V` |
| Paste Special — Inhalte einfügen | `Ctrl+Alt+V` | `Cmd+Alt+V` |

### Zeichenformatierung

Quelle: [format](../../crates/engine/src/cmd/format.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Bold — Fett | `Ctrl+B` | `Cmd+B` |
| Italic — Kursiv | `Ctrl+I` | `Cmd+I` |
| Underline — Unterstreichen | `Ctrl+U` | `Cmd+U` |
| Double Underline — Doppelt unterstreichen | `Ctrl+Shift+D` | `Cmd+Shift+D` |
| Underline Words Only — Nur Wörter unterstreichen | `Ctrl+Shift+W` | `Cmd+Shift+W` |
| Strikethrough — Durchstreichen | `Ctrl+Shift+X` | `Cmd+Shift+X` |
| Subscript — Tiefstellen | `Ctrl+=` | `Cmd+=` |
| Superscript — Hochstellen | `Ctrl+Shift+=` | `Cmd+Shift+=` |
| All Caps — Großbuchstaben | `Ctrl+Shift+A` | `Cmd+Shift+A` |
| Small Caps — Kapitälchen | `Ctrl+Shift+K` | `Cmd+Shift+K` |
| Hidden — Ausgeblendeter Text | `Ctrl+Shift+H` | `Cmd+Shift+H` |
| Font — Schriftart | `Ctrl+Shift+F` | `Cmd+Shift+F` |
| Font Size — Schriftgröße | `Ctrl+Shift+P` | `Cmd+Shift+P` |
| Increase Font Size — Schrift vergrößern | `Ctrl+Shift+. / Ctrl+Shift+>` | `Cmd+Shift+. / Cmd+Shift+>` |
| Decrease Font Size — Schrift verkleinern | `Ctrl+Shift+, / Ctrl+Shift+<` | `Cmd+Shift+, / Cmd+Shift+<` |
| Grow Font 1 Point — Schrift um einen Punkt vergrößern | `Ctrl+]` | `Cmd+]` |
| Shrink Font 1 Point — Schrift um einen Punkt verkleinern | `Ctrl+[` | `Cmd+[` |
| Text Highlight Color — Text hervorheben | `Ctrl+Alt+H` | — |
| Change Case — Groß-/Kleinschreibung ändern | `Shift+F3` | `Shift+F3` |
| Reset Character Formatting — Zeichenformatierung zurücksetzen | `Ctrl+Space` | `Ctrl+Space` |
| Font Dialog — Schriftartdialog | `Ctrl+D` | `Cmd+D` |

### Absätze und Formatvorlagen

Quelle: [para](../../crates/engine/src/cmd/para.rs), [inspector](../../crates/engine/src/cmd/inspector.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Style Inspector — Formatvorlagen prüfen | `Shift+F1` | `Shift+F1` |
| Align Left — Linksbündig | `Ctrl+L` | `Cmd+L` |
| Center — Zentriert | `Ctrl+E` | `Cmd+E` |
| Align Right — Rechtsbündig | `Ctrl+R` | `Cmd+R` |
| Justify — Blocksatz | `Ctrl+J` | `Cmd+J` |
| Distributed — Verteilte Ausrichtung | `Ctrl+Shift+J` | `Cmd+Shift+J` |
| Increase Indent — Einzug vergrößern | `Ctrl+M` | `Cmd+M` |
| Decrease Indent — Einzug verkleinern | `Ctrl+Shift+M` | `Cmd+Shift+M` |
| Hanging Indent — Hängenden Einzug vergrößern | `Ctrl+T` | `Cmd+T` |
| Reduce Hanging Indent — Hängenden Einzug verkleinern | `Ctrl+Shift+T` | `Cmd+Shift+T` |
| Single Spacing — Einfacher Zeilenabstand | `Ctrl+1` | `Cmd+1` |
| Double Spacing — Doppelter Zeilenabstand | `Ctrl+2` | `Cmd+2` |
| 1.5 Line Spacing — 1,5-facher Zeilenabstand | `Ctrl+5` | `Cmd+5` |
| Add Space Before Paragraph — Abstand vor Absatz umschalten | `Ctrl+0` | `Cmd+0` |
| Reset Paragraph Formatting — Absatzformatierung zurücksetzen | `Ctrl+Q` | `Ctrl+Q` |
| Normal Style — Standardformatvorlage | `Ctrl+Shift+N` | `Cmd+Shift+N` |
| Heading 1 — Überschrift 1 | `Ctrl+Alt+1` | `Cmd+Alt+1` |
| Heading 2 — Überschrift 2 | `Ctrl+Alt+2` | `Cmd+Alt+2` |
| Heading 3 — Überschrift 3 | `Ctrl+Alt+3` | `Cmd+Alt+3` |
| Bullets — Aufzählung | `Ctrl+Shift+L` | `Cmd+Shift+L` |
| Move Paragraph Up — Absatz nach oben verschieben | `Alt+Shift+Up` | `Ctrl+Shift+Up` |
| Move Paragraph Down — Absatz nach unten verschieben | `Alt+Shift+Down` | `Ctrl+Shift+Down` |
| Promote — Ebene höher | `Alt+Shift+Left` | `Ctrl+Shift+Left` |
| Demote — Ebene tiefer | `Alt+Shift+Right` | `Ctrl+Shift+Right` |
| Styles Pane — Formatvorlagenbereich | `Ctrl+Alt+Shift+S` | `Cmd+Alt+Shift+S` |
| Apply Styles — Formatvorlage anwenden | `Ctrl+Shift+S` | — |

### Einfügen

Quelle: [insert](../../crates/engine/src/cmd/insert.rs), [equation](../../crates/engine/src/cmd/equation.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Equation — Gleichung einfügen | `Alt+=` | `Alt+=` |
| Link — Link einfügen | `Ctrl+K` | `Cmd+K` |
| Bookmark — Textmarke einfügen | `Ctrl+Shift+F5` | `Cmd+Shift+F5` |
| Field — Feld einfügen | `Ctrl+F9` | `Cmd+F9` |
| Insert Date Field — Datumsfeld einfügen | `Alt+Shift+D` | `Ctrl+Shift+D` |
| Insert Time Field — Uhrzeitfeld einfügen | `Alt+Shift+T` | `Ctrl+Shift+T` |
| Insert Page Field — Seitenzahlfeld einfügen | `Alt+Shift+P` | `Ctrl+Shift+P` |

### Ansichten und Gliederung

Quelle: [view](../../crates/engine/src/cmd/view.rs), [outline](../../crates/engine/src/cmd/outline.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Show/Hide ¶ — Formatierungszeichen anzeigen/ausblenden | `Ctrl+Shift+8 / Ctrl+8` | `Cmd+Shift+8 / Cmd+8` |
| Print Layout — Drucklayout | `Ctrl+Alt+P` | `Cmd+Alt+P` |
| Draft — Entwurfsansicht | `Ctrl+Alt+N` | `Cmd+Alt+N` |
| Outline — Gliederungsansicht | `Ctrl+Alt+O` | `Cmd+Alt+O` |
| Split — Ansicht teilen | `Ctrl+Alt+S` | `Cmd+Alt+S` |
| Full Screen — Vollbild | `F11` | `Cmd+Ctrl+F / F11` |
| Expand — Gliederung aufklappen | `Alt+Shift+=` | `Alt+Shift+=` |
| Collapse — Gliederung einklappen | `Alt+Shift+-` | `Alt+Shift+-` |
| Show Level 1 — Bis Gliederungsebene 1 anzeigen | `Alt+Shift+1` | `Alt+Shift+1` |
| Show Level 2 — Bis Gliederungsebene 2 anzeigen | `Alt+Shift+2` | `Alt+Shift+2` |
| Show Level 3 — Bis Gliederungsebene 3 anzeigen | `Alt+Shift+3` | `Alt+Shift+3` |
| Show Level 4 — Bis Gliederungsebene 4 anzeigen | `Alt+Shift+4` | `Alt+Shift+4` |
| Show Level 5 — Bis Gliederungsebene 5 anzeigen | `Alt+Shift+5` | `Alt+Shift+5` |
| Show Level 6 — Bis Gliederungsebene 6 anzeigen | `Alt+Shift+6` | `Alt+Shift+6` |
| Show Level 7 — Bis Gliederungsebene 7 anzeigen | `Alt+Shift+7` | `Alt+Shift+7` |
| Show Level 8 — Bis Gliederungsebene 8 anzeigen | `Alt+Shift+8` | `Alt+Shift+8` |
| Show Level 9 — Bis Gliederungsebene 9 anzeigen | `Alt+Shift+9` | `Alt+Shift+9` |
| Show All Levels — Alle Gliederungsebenen anzeigen | `Alt+Shift+A` | `Alt+Shift+A` |
| Show First Line Only — Nur erste Zeile anzeigen | `Alt+Shift+L` | — |

### Verweise und Felder

Quelle: [references](../../crates/engine/src/cmd/references.rs), [fields](../../crates/engine/src/cmd/fields.rs), [index](../../crates/engine/src/cmd/index.rs), [citations](../../crates/engine/src/cmd/citations.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Mark Entry — Indexeintrag markieren | `Alt+Shift+X` | — |
| Field Codes — Feldfunktionen anzeigen | `Alt+F9` | `Alt+F9` |
| Toggle Field Codes — Feldfunktion umschalten | `Shift+F9` | `Shift+F9` |
| Unlink Fields — Feldverknüpfung aufheben | `Ctrl+Shift+F9` | `Cmd+Shift+F9` |
| Lock Fields — Felder sperren | `Ctrl+F11` | `Cmd+F11` |
| Unlock Fields — Felder entsperren | `Ctrl+Shift+F11` | `Cmd+Shift+F11` |
| Next Field — Nächstes Feld | `Alt+F1` | `Alt+F1` |
| Previous Field — Vorheriges Feld | `Shift+F11 / Alt+Shift+F1` | `Shift+F11 / Alt+Shift+F1` |
| Mark Citation — Zitat für Rechtsgrundlagenverzeichnis markieren | `Alt+Shift+I` | — |
| Insert Footnote — Fußnote einfügen | `Ctrl+Alt+F` | `Cmd+Alt+F` |
| Insert Endnote — Endnote einfügen | `Ctrl+Alt+D` | `Cmd+Alt+E` |
| Next Footnote — Nächste Fußnote | `Alt+Shift+> / Alt+Shift+.` | — |
| Update Field — Feld aktualisieren | `F9` | `F9` |

### Überprüfen

Quelle: [review](../../crates/engine/src/cmd/review.rs), [speech](../../crates/engine/src/cmd/speech.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Read Aloud — Vorlesen | `Ctrl+Alt+Space` | `Cmd+Alt+Space` |
| New Comment — Neuer Kommentar | `Ctrl+Alt+M` | `Cmd+Alt+A / Cmd+Alt+M` |
| Track Changes — Änderungen nachverfolgen | `Ctrl+Shift+E` | `Cmd+Shift+E` |
| Word Count — Wörter zählen | `Ctrl+Shift+G` | — |
| Spelling & Grammar — Rechtschreibung und Grammatik | `F7 / Alt+F7` | `F7 / Alt+F7 / Cmd+Alt+L` |
| Thesaurus — Synonyme | `Shift+F7` | `Shift+F7` |

### Seriendruck

Quelle: [mailings](../../crates/engine/src/cmd/mailings.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Edit Recipient List — Empfängerliste bearbeiten | `Alt+Shift+E` | — |
| Insert Merge Field — Seriendruckfeld einfügen | `Alt+Shift+F` | — |
| Preview Results — Seriendruckvorschau | `Alt+Shift+K` | — |
| Finish & Merge — Seriendruck abschließen | `Alt+Shift+N` | — |
| Print Documents — Seriendokumente drucken | `Alt+Shift+M` | — |

### Weitere Werkzeuge

Quelle: [tools/specs](../../crates/engine/src/cmd/tools/specs.rs).

| Befehl / Bedeutung | Windows / Linux / BSD | macOS |
|---|---|---|
| Repeat — Letzte Aktion wiederholen | `F4` | `F4` |
| Macros — Makros | `Alt+F8` | `Alt+F8` |
| Search — Befehl suchen | `Alt+Q` | — |
| AutoText — AutoText einfügen | `Alt+F3` | — |
| Link to Previous — Mit vorherigem Abschnitt verknüpfen | `Alt+Shift+R` | — |
| Extend Selection — Auswahl erweitern | `F8` | `F8` |

## Kontextabhängige Bedienung

Diese Steuerungen ergänzen das Befehlsverzeichnis. Ein fokussierter Dialog, ein Menü, ein ausgewähltes
Objekt oder ein aktives Werkzeug kann Vorrang vor normaler Textbearbeitung haben.

| Kontext | Tasten / Geste | Wirkung |
|---|---|---|
| Textnavigation | Shift + Bewegungskürzel | Erweitert die Auswahl, sofern die Kombination nicht ausdrücklich anders belegt ist |
| Ausgewählter Text | Shift+Delete | Schneidet die Auswahl aus; ohne Auswahl keine Wirkung |
| Dokument | Shift+Backspace | Löscht rückwärts |
| Dokument | Insert | Schaltet Überschreiben nur um, wenn unter **File › Options › Advanced** aktiviert |
| Menüband | F10; auf Nicht-macOS Alt kurz drücken und loslassen | Zeigt KeyTips; den eingeblendeten Buchstaben folgen. Escape geht zurück |
| Menüband | Ctrl+F1 (Nicht-macOS); Cmd+Alt+R (macOS) | Klappt das Menüband ein/aus; eine eigene Ctrl+F1-Zuordnung hat Vorrang |
| Dokument | Shift+F10 | Öffnet das Kontextmenü |
| Read Mode | PageDown / Right; PageUp / Left | Nächste/vorherige Leseseite |
| Read Mode / Vollbildansichten | Escape | Beendet die Ansicht, nachdem nähere Kontexte wie Auswahl, Werkzeuge oder Menüs Escape verarbeitet haben |
| Gleichungsbearbeitung | Pfeile, Home, End | Bewegt innerhalb der Gleichung |
| Gleichungsbearbeitung | Tab / Shift+Tab | Nächster/vorheriger Gleichungsteil |
| Gleichungsbearbeitung | Backspace / Delete; Enter; Escape | Löscht in der Gleichung; baut die Gleichung auf oder beginnt einen neuen Absatz, wenn nichts aufzubauen ist; beendet die Gleichungsbearbeitung |
| Text in Textfeld / WordArt | Escape | Wählt das umgebende Objekt; Delete kann es anschließend entfernen |
| Kopf-/Fußzeile ohne Textauswahl | Escape | Kehrt zum Hauptdokument zurück |
| Formatübertragung, Stift/Radierer oder Tabellenzeichnen | Escape | Beendet das aktive Werkzeug |

Quellen: [Kürzelverarbeitung](../../crates/ui-egui/src/keys/shortcuts.rs),
[Dokumenteingabe](../../crates/ui-egui/src/keys/canvas.rs), [KeyTips](../../crates/ui-egui/src/keytips.rs).

## Weitere Hilfe und Abdeckung

| Thema | Vorhandene Dokumentation |
|---|---|
| Wiederherstellung und ungespeicherte Arbeit | [Recovery](../recovery.md) |
| Sprachprüfung einrichten | [Wörterbücher](../spelling-dictionaries.md) |
| Literaturverwaltung | [Zotero](../zotero.md) |
| Formate, Layout und Schreibverhalten | [Dateiformate](../file-format-parity.md), [Layout](../layout-parity.md), [Schreibverhalten](../typing-parity.md) |
| Oberfläche und Plattformen | [UI](../ui-parity.md), [Lokalisierung](../localization-parity.md), [Hardware/Plattformen](../hardware-parity.md) |
| Automatisierung | [MCP](../mcp.md), [Steuerprotokoll](../control-protocol.md), [CLI-Beispiele im README](../../README.md#for-agents-cli-and-mcp) |
| Bekannte Lücken und geplante Arbeit | [Lücken](../gaps.md), [Roadmap](../roadmap.md) |

Dies ist ein erstes Handbuch, noch keine vollständige Anleitung für jeden Befehl. Ausführliche
Abläufe für komplexe Tabellen und Objekte, Zeichnen, Gleichungen, Dokumentenschutz, Makros und jeden
einzelnen Dialog müssen ergänzt werden. Die Kürzeltabellen decken alle Standardbelegungen des
geprüften Engine-Verzeichnisses und die oben aufgeführten Kontextsteuerungen ab; weitere Objekt-,
Werkzeug- und Dialoggesten benötigen noch einen systematischen Durchgang. Vorhandene Themenseiten
bleiben für ihre Details maßgeblich und sind derzeit überwiegend englisch. Geplante Fähigkeiten
in Roadmap-/Paritätsseiten sind keine Bedienungsanleitung für bereits verfügbare Funktionen.

Für Beitragende: Englische Anleitung und vorhandene Übersetzungen synchron halten und das Verhalten am Quellcode prüfen;
siehe [AGENTS.md](../../AGENTS.md#user-documentation).
