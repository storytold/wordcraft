# Segmente und Seitenbeschriftung

> **Last updated:** 2026-10-31 · External-edit protection and table-boundary guidance.

Unter **Insert › Header & Footer › Page Number › Format Page Numbers** öffnest du die
Seitennummerierung. Oben wählst du das zu bearbeitende Segment. Unten legt **+ New segment…** ein
Segment ab der aktuellen Dokumentseite oder ab dem Textcursor auf einer neuen Seite an. Es reicht
bis zum nächsten vorhandenen Abschnitt oder zum Dokumentende. Vorhandene Grenzen werden bearbeitet,
nicht doppelt eingefügt.
Bei Tabellen setzt du den Textcursor davor oder dahinter; Segmente können eine Tabelle nicht teilen.

Mit **Custom page labels** vergibst du einen Segmentnamen und wählst die Beschriftung:

| Modus | Beispiel |
|---|---|
| No label – keine Beschriftung | Cover ohne C oder 0 |
| Text only – nur Text | Auf jeder Seite „Anhang“ |
| Counter only – nur Zähler | `0, 1, 2`, `I, II, III` oder `A, B, C` |
| Text and counter – Text und Zähler | `TOC I`, `Anlage 0` oder `Anhang A` |

Der Segmentname in der Auswahl ist unabhängig vom gedruckten Text. Das Textfeld wird wörtlich
verwendet; das Trennzeichen ist frei wählbar. Römische und alphabetische Formate gibt es in Groß-
und Kleinschreibung. **Start at** erlaubt 0, 1 oder einen anderen ganzzahligen Startwert bis 100.000.
Alternativ setzt du den Zähler des vorherigen nummerierten Segments fort. Reine Textsegmente und
Segmente ohne Beschriftung erhöhen diesen Zähler nicht. Beim Formatwechsel bleibt der numerische
Zählstand erhalten. Eine lediglich ausgeblendete Beschriftung innerhalb eines nummerierten
Abschnitts ist davon zu unterscheiden.

Römische Zahlen und Buchstaben haben kein Nullzeichen. Bei Start 0 bestimmt **Zero-page label**
die vollständige Beschriftung der ersten Seite: leer oder beispielsweise „Auftakt“. Danach folgen
I, II beziehungsweise A, B. Die Vorschau zeigt drei Beschriftungen. Erst **OK** beziehungsweise
**Create segment** übernimmt die Änderung; **Cancel** verwirft den Entwurf. Die übernommene Änderung
lässt sich rückgängig machen.

Vorhandener Kopf-/Fußzeilentext bleibt erhalten. Die Funktion kopiert die wirksamen Kopf- und
Fußzeilen für das Segment, damit vorherige oder nachfolgende verknüpfte Abschnitte unverändert
bleiben. Fehlt in einer aktiven Kopf-/Fußzeilenvariante ein PAGE-Feld, wird es in der Fußzeile ergänzt.
Abweichende erste und gerade Seiten werden berücksichtigt. Durch Abschalten von **Custom page
labels** kehrst du zur gewöhnlichen Abschnittsnummerierung zurück.

Ein Buch kann beispielsweise diese Segmente verwenden: Cover ohne Beschriftung; Inhaltsverzeichnis
mit „TOC“ und römischen Zahlen ab I; Vorwort mit Buchstaben ab A; Haupttext mit Zahlen ab 1. Zusätzliche
Vorspannseiten verschieben den ausdrücklichen Zählbeginn des Haupttexts nicht.

Aktualisiere Inhaltsverzeichnis und andere erzeugte Verzeichnisse nach Dokumentänderungen. Ihre
Seitenverweise verwenden die vollständige Beschriftung, bei leerem Label den Segmentnamen.
Wiederholte Beschriftungen verändern die eindeutigen Sprungziele nicht. Physische Seitenpositionen
und Gesamtseitenzahl bleiben für Navigation und Druck erhalten.

JSON und DOCX speichern die Segmentregeln. DOCX enthält normale Text-, PAGE- und IF-Felder als
Ersatzdarstellung. Zusätzlich liegen die Regeln in normalen Dokumenteigenschaften, verknüpft über
kurze Kennungen an den erzeugten Inhaltssteuerelementen. Daraus stellt WordCraft die bearbeitbaren
PAGE-Felder auch ohne seine ignorierbaren Erweiterungsattribute wieder her. Die Zuordnung folgt
den Kopf-/Fußzeilen auch bei verschobenen oder neu verknüpften Abschnitten. Lange Namen und Texte
werden auf begrenzte Eigenschaftswerte verteilt. Automatisierte Tests prüfen diese Wiederherstellung,
erneutes Speichern und den Erhalt eigener Dokumenteigenschaften.
Ein gemeinsam mit dem Nutzer durchgeführter Word-Test unter Windows (30.10.2026) bestätigt den
Rückimport nach zusätzlichen Seitenumbrüchen und Texteingaben. Word entfernte die ignorierbaren
Attribute, behielt aber alle 13 Sicherungseigenschaften unverändert. WordCraft stellte sämtliche
sechs Segmentregeln einschließlich des langen Unicode-Namens wieder her: Cover ohne Beschriftung,
dreimal „Vorwort“, „TOC I–III“, „A–C“, „1–3“ und „Annex 0–2“. Die erste Inhaltsseite zeigt wieder
„TOC I“ statt Words gespeichertem „TOC III“. Erneutes DOCX-Speichern und Öffnen erhält alle
Abschnittseinstellungen. Die Datei hat in beiden Programmen 16 Seiten: Eine zusätzliche Leerseite
nach manuellem Seitenumbruch und folgendem leeren Absatz mit Abschnittswechsel wurde korrigiert.
Das bestätigt diesen synthetischen Testfall, nicht die allgemeine Layoutgleichheit.
Bewahre die WordCraft-Quelldatei auf: Wer die Dokumenteigenschaften oder erzeugten
Inhaltssteuerelemente entfernt, löscht auch die Wiederherstellungsinformationen. Neue Exporte sichern zusätzlich einen Prüfwert der erzeugten Beschriftung und Feldanweisung.
Bei erkannten Änderungen öffnet WordCraft das gesamte Dokument mit seinen normalen Word-Texten
und -Feldern und stellt die Segmentregeln nicht automatisch wieder her. So bleiben auch benachbarte
Beschriftungen erhalten. Benutzerdefinierte Segmente können anschließend in WordCraft neu eingerichtet
werden. Aktualisierte Feldresultate, aufgeteilte Textläufe und Formatierungen werden beim Vergleich
ignoriert; manuell geänderte Feldresultate sind von normalen Aktualisierungen nicht unterscheidbar.
Ältere Exporte ohne Prüfwert behalten das bisherige Wiederherstellungsverhalten. Das erhält Änderungen,
übersetzt sie aber nicht automatisch in Segmentregeln. Andere Programme sind noch nicht geprüft.
Das Fortsetzen unter Überspringen nicht zählender
Segmente ist WordCraft-spezifisch. PDF-Ausgabe und PDF-Seitenlabels verwenden die
gewählten Beschriftungen, auch bei Start 0. Andere Exportformate erhalten die Segmentmetadaten
derzeit nicht.

Neue Oberflächentexte verwenden den Übersetzungskatalog und erscheinen ohne Übersetzung auf
Englisch. Das Beispiel für den Befehl `layout.pageSegment` steht in der
[englischen Anleitung](page-label-segments.md). Ohne `new` bearbeitet er den aktuellen Abschnitt
oder den expliziten, bei 0 beginnenden `section`-Index. Ohne `format` oder `start` bleiben die
bisherigen Werte erhalten. Ein ausdrückliches `start:null` setzt die Zählung fort;
`label:null` stellt gewöhnliche Seitenlabels wieder her. Namen, Text und Nullseitenlabel sind auf
256 Zeichen begrenzt, Trennzeichen auf 16; Steuerzeichen werden zurückgewiesen.

Für eine unabhängige Kopf- oder Fußzeile öffnest du sie auf der gewünschten Seite und schaltest
**Insert › Header & Footer › Link to Previous** aus. Der Inhalt einschließlich verankerter
Textfelder wird kopiert. Andere Abschnitte behalten ihren Inhalt. Erneutes Einschalten übernimmt
wieder die Zeile des vorherigen Abschnitts. Die Schaltfläche betrifft die aktuelle Variante
(Standard, erste oder gerade Seite), ist im ersten Abschnitt deaktiviert und unterstützt Rückgängig.
Eine absichtlich leere entkoppelte Kopf-/Fußzeile bleibt beim Schließen und Speichern leer.
Eine eigene Kontextregisterkarte für Kopf- und Fußzeilen ist hier noch nicht enthalten.

Bei einem Segmentstart innerhalb einer Tabelle zeigt der Dialog einen Hinweis und deaktiviert
„Create segment“. Setze den Cursor vor oder hinter die Tabelle und wähle den Start am Cursor.

## Revision history

| Date | Change |
|---|---|
| 2026-10-31 | Documented guarded exports, conservative conflict import and table-boundary guidance. |
