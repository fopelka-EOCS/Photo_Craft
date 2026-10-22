# PicsRoom iOS — experimental PhotoCraft client

This proposal adds the existing PicsRoom Build 13 iOS application as an isolated Xcode project. It uses SwiftUI, Core Image, Vision/Core ML and a Rust FFI bridge. This is **not** a pure-Rust UI and does not satisfy the current upstream architecture policy; it is offered for maintainer discussion, not as a merge-ready replacement for PhotoCraft.

## Reproduce the build

1. On macOS install Xcode, Python 3 and Rust 1.95.0.
2. From the repository root run `python3 packaging/picsroom-ios/scripts/prepare.py`. This extracts the tested PhotoCraft engine revision from Git history and downloads the two Core ML packages at pinned revisions, verifying every file against the bundled SHA-256 manifest. Neither the duplicate engine tree nor model weights are committed.
3. Run `rustup target add --toolchain 1.95.0 aarch64-apple-ios aarch64-apple-ios-sim`.
4. Open `packaging/picsroom-ios/PhotoCraft.xcodeproj`, select the PhotoCraft scheme and an iPhone simulator. For a physical device select your own signing team and unique bundle identifier. No personal signing team is included.

The bridge remains pinned to `8d840eab5962b41e4e15b115107eef8384648fb5`, the version used for the tested application. Adapting it to current main and the engine-first command architecture is follow-up work. The nested bridge is its own Cargo workspace; the desktop/web workspace is unchanged. Personal photographs, credentials and build products are excluded.

## Review scope and validation

The application supports photo/RAW development, local masks, presets and reference-look matching, cropping, batch selection/export, and optional opt-in API image expansion. Reference matching is statistical, not reconstruction of a Lightroom recipe. Live paid image expansion has not been tested. See VALIDATION.md for historical test results and their limitations; those results describe the original standalone build, not certification of upstream compatibility.

The remaining documentation below describes the standalone app and its German interface.

---

# PicsRoom für iPhone und iPad – Version 1.0, Build 12

Native private iPhone-/iPad-App mit PhotoCraft-Projektkern und einer GPU-gestützten Core-Image-Entwicklung. Die Oberfläche orientiert sich an den bereitgestellten Lightroom-Referenzen: transparente Bedienfelder, schmale Regler mit Ringgriff, größere graue Symbole und eine blaue Markierung des aktiven Hauptwerkzeugs. Die Hauptleiste enthält Aktionen, Looks, Retusche, Zuschnitt, Bearbeiten und Maskieren; unter Bearbeiten liegt die scrollbare Werkzeugauswahl.

## Neu in Build 12: Zuschnitt, Formate und KI-Erweitern

**Zuschnitt → Format:** frei oder Original-Seitenverhältnis, 1:1, 1:2, 2:3, 3:4, 4:5, 5:7, 8,5:11, 9:16 und 10:16. Hoch-/Querformat lassen sich umschalten. Das Schloss hält das Verhältnis beim Ziehen fest; vier Eck- und vier Kantengriffe ändern den Rahmen, Ziehen innerhalb verschiebt ihn. Instagram bietet Quadrat, 4:5 und Story/Reel 9:16; TikTok bietet 9:16. Die Vorlagen bestimmen das Seitenverhältnis, sie veröffentlichen nichts auf einer Plattform.

„Original“ setzt das ursprüngliche Seitenverhältnis unter Berücksichtigung von 90°-Drehungen; es stellt nicht bereits abgeschnittene Bildteile wieder her. Dafür Rückgängig verwenden. **Anwenden** übernimmt die Auswahl, **Auswahl verwerfen** verwirft nur den noch nicht angewendeten Rahmen. Drehen, Spiegeln und die Regler unter **Geometrie** werden direkt gespeichert und können über Rückgängig zurückgenommen werden. Im Format-Tab wird Geometrie für den Zuschnitt vorübergehend ausgeblendet. Im Geometrie-Tab zeigt die Vorschau die gespeicherten Korrekturen.

Der breite Kopfbereich über den Lichtreglern entfällt. Der Ziehgriff und das Bedienfeld sind nochmals niedriger; die Regler bleiben scrollbar. Zurücksetzen liegt unter **Weitere Aktionen → Anpassungen zurücksetzen**, der Vorher-Vergleich bleibt über Gedrückthalten des Fotos verfügbar.

### KI-Erweitern einrichten

Unter **Zuschnitt → Format** die horizontale Werkzeugleiste nach links wischen und **KI-Erweitern** öffnen. Einen eigenen OpenAI-API-Schlüssel im geschützten Eingabefeld eintragen; er wird nur im iOS-Schlüsselbund dieses Geräts gespeichert, nicht in Projekten oder im Quellcode. Schlüssel lassen sich dort wieder entfernen. Der Nutzer benötigt einen für das Bildmodell berechtigten API-Zugang und gegebenenfalls Guthaben/Freischaltung.

Zielformat und zusätzlichen Rand wählen, optional eine Beschreibung ergänzen, die Übertragung des Fotos bestätigen und **Mit KI erweitern** starten. Erst dabei wird eine Anfrage an OpenAI gesendet; es können API-Kosten anfallen. Die App nutzt `gpt-image-2` über `https://api.openai.com/v1/images/edits`, Qualitätsstufe medium, eine PNG-Eingabe und eine transparente Erweiterungsmaske. Umleitungen werden nicht verfolgt, es gibt keine automatische Wiederholung kostenpflichtiger Anfragen. Abbrechen beendet den lokalen Aufruf; bereits begonnene Verarbeitung beim Anbieter kann trotzdem berechnet werden.

Das Ergebnis hat maximal 1536 Pixel an der langen Kante. Die Ausgabeabmessungen werden vorab angezeigt und auf 16-Pixel-Schritte gerundet. Der vorhandene Bildbereich wird in dieser Ausgabeauflösung lokal wieder eingesetzt, damit das Modell ihn nicht verändert. Nur der zusätzliche Rand stammt aus der Generierung. Das Ergebnis prüfen und **Als neues Bild übernehmen** oder verwerfen. Bei Übernahme wird ein neues PNG-basiertes Projekt ohne RAW-Eigenschaften angelegt; das bisherige Projekt und dessen Original bleiben erhalten. Es gibt keine Offline-KI-Generierung und keinen mitgelieferten API-Schlüssel.

**Prüfgrenze:** Vorbereitung, Maskentransparenz, Request-Schema, Rückgabezusammensetzung, Fehlermeldungen und der gesperrte Start ohne Zugang werden lokal mit einem simulierten Dienst geprüft. Ohne eingerichteten API-Zugang wurde kein echter kostenpflichtiger OpenAI-Aufruf durchgeführt; die Qualität der Generierung und die Freischaltung des persönlichen Kontos sind damit nicht bestätigt.

Technische Referenzen: [OpenAI Images Edit API](https://developers.openai.com/api/reference/resources/images/methods/edit), [Bildgenerierung](https://developers.openai.com/api/docs/guides/image-generation).

## Neu in Build 11: Export und kompakter Editor

Das Teilen-Symbol öffnet **Exportieren**. Ein oder mehrere Projekte über die Vorschaubilder auswählen; bei einer geöffneten Fotoauswahl werden deren Bilder angeboten, sonst die gespeicherten Projekte. Das aktuelle Bild ist vorausgewählt.

- Dateityp: JPG, PNG, TIFF (16 Bit), HDR-HEIF (10 Bit), unverändertes Original oder PicsRoom-Projekt.
- Größe: volle Auflösung, 2048 oder 4096 Pixel lange Kante oder ein eigener Wert von 256 bis 12000 Pixeln. Das Seitenverhältnis bleibt erhalten; es wird nicht hochskaliert.
- JPEG-Qualität: 60, 75, 85, 95 oder 100 Prozent. PNG/TIFF sind verlustfrei; HDR verwendet den Systemencoder ohne zusätzlichen Qualitätsregler.
- Optionales Textwasserzeichen: unten rechts, weiß mit dunkler Kontur, proportional zur Ausgabegröße; maximal 80 Zeichen. Die Vorschau im Editor bleibt unverändert.
- Weitere Optionen: Dateiname mit laufender Nummer, sRGB oder Display P3 für gerenderte SDR-Formate. HDR bleibt HLG/Rec.2020. Gerenderte Exporte übernehmen keine ursprünglichen Kamera-/Standort-Metadaten.
- **Teilen / In Dateien sichern** öffnet die iOS-Freigabe für alle ausgewählten Dateien. **In Fotos speichern** fügt gerenderte Bilder direkt zur Fotomediathek hinzu und fragt dafür die Hinzufügen-Berechtigung an. Original-/Projektdateien über die Freigabe in Dateien sichern.
- Originale und Projekte erhalten keine Größen-, Qualitäts- oder Wasserzeichenänderungen. Originaldateien behalten ihre Metadaten. Alte Projekte ohne gespeichertes Original melden dies beim Originalexport.
- Der Stapelexport kehrt anschließend zum zuvor geöffneten Foto zurück. Bei einem Fehler werden keine unvollständigen Ergebnisse geteilt.

Die Oberfläche hat kleinere Symbole, Reglergriffe und Beschriftungen sowie ein niedrigeres transparentes Bedienfeld, das sich vertikal scrollen lässt. Die zusätzlichen Regler bleiben durch Scrollen erreichbar. Die vorhandene Geste zum vollständigen Ausblenden und der Vorher-Vergleich bleiben erhalten. Rückgängig bleibt oben; Wiederholen und erneutes Speichern liegen im Menü **Weitere Aktionen**.

Adobe-Cloud-Freigabelinks, C2PA/Content Credentials und Adobe-spezifische Social-Media-Aktionen sind nicht implementiert. Teilen mit installierten Apps erfolgt über das iOS-Teilen-Menü.

## Neu in Build 10

- Die rote Maskenüberlagerung verschwindet beim Anfassen oder Ändern eines lokalen Reglers. Das Auge blendet sie wieder ein; beim Wechseln oder Zeichnen einer Maske wird sie erneut sichtbar.
- Mehrere Fotos über die Fotoauswahl importieren (bis zu 30, nacheinander zur Speicherbegrenzung). In der Mediathek öffnet **Auswählen → Fertig** die markierten Projekte; oben wechseln Pfeile zwischen ihnen. Die Bearbeitungen bleiben pro Foto gespeichert. Die separate Stapelbearbeitung bleibt verfügbar.
- Neuer sichtbarer App-Name **PicsRoom**. Bundle-ID, Xcode-Projekt/Schema PhotoCraft und `.pcraft` bleiben für vorhandene Projekte und Updates kompatibel.
- Unter **Looks → Eigene Presets** aktuelle Licht-/Farbentwicklung, HSL, Kurven und Grading benennen und speichern, anwenden, teilen oder löschen. Original, RAW-Weißabgleich, Masken, Reparaturen und Zuschnitt sind nicht Teil eines Presets.
- Eigene Presets als `.picsroompreset` weitergeben/importieren. Lightroom-Presets als **einzelne XMP-Dateien** importieren. Unterstützt werden Exposure2012, Contrast2012, Highlights2012, Shadows2012, Whites2012, Blacks2012, Saturation, Vibrance, Texture, Clarity2012 und Dehaze. Nur enthaltene unterstützte Werte ändern das aktuelle Foto. Werte werden auf gültige Reglerbereiche begrenzt.
- Lightroom verwendet eine andere Bildentwicklung: Ergebnisse sind angenähert. Profile/LUTs, Kurven, HSL, Weißabgleich, Masken und weitere Adobe-Effekte werden beim XMP-Import nicht übernommen. DNG-/LRTemplate-Presets werden nicht als Presets unterstützt; DNG-Fotos bleiben als RAW importierbar. ZIP-Presetpakete zuerst entpacken. Eine Datei ohne unterstützte Werte wird abgelehnt.

## Neu in Build 9: RAW und erweiterte Entwicklung

**RAW öffnen:** Über Mediathek → Dateien eine `.CR3` der Canon EOS 250D oder ein Apple-ProRAW-`.DNG` wählen. Die Fotoauswahl fordert die aktuelle Datei ohne automatische Formatkonvertierung an; bei RAW/JPEG-Paaren oder bereits bearbeiteten Fotos ist der Dateiimport des Originals der eindeutige Weg. Im Titel erscheint **RAW**, wenn tatsächlich Rohdaten entwickelt werden. Die App verwendet Apples `CIRAWFilter`, keine eingebettete JPEG-Vorschau. Unterstützte Kameras hängen von der iOS-Version ab; die EOS 250D steht auf Apples Liste für iOS 27. Nicht unterstützte RAW-Dateien werden mit einer Fehlermeldung abgelehnt.

Das unveränderte Original ist als komprimierter, inhaltsadressierter Binärinhalt im `.pcraft` enthalten. Es wird beim Teilen des Projekts mitgenommen und lässt sich über **Unverändertes Original** wieder exportieren. Auch neu importierte JPEG-/HEIC-/TIFF-Originale bleiben erhalten. Die bisherige Rasterebene dient als Kompatibilitätsvorschau; die neue Entwicklung und der Export verwenden das Original. Alte Projekte ohne Original bleiben bearbeitbar, erhalten dadurch aber keine nachträglichen RAW-Daten oder zusätzliche Farbtiefe.

Unter **Farbe** gibt es für RAW Temperatur in Kelvin, RAW-Tönung und **Weißabgleich wie aufgenommen**. Der Belichtungsregler steuert bei RAW den Decoder. Die bisherigen Wärme-/Tönungsregler wirken zusätzlich; die Pipette ist ausdrücklich eine RGB-Pipette. Der gewählte Decoder wird gespeichert; auf Geräten ohne diesen Decoder erscheint ein Fehler statt eines stillschweigend veränderten Looks.

**Qualität und Export:** Die Original-Verarbeitung arbeitet mit Halbfloat-Zwischenbildern und erweitertem Farbumfang. JPEG/PNG bleiben sRGB/8-Bit-SDR. Neu sind **TIFF mit 16 Bit pro Kanal in Display P3** und **10-Bit-HEIF in HLG/Rec.2020**. HDR-Export kann vorhandenen Dynamikumfang erhalten, erzeugt keine verlorenen Bilddetails aus einem JPEG. Die Editorvorschau ist derzeit SDR/Display P3; sie ist keine verbindliche HDR-Helligkeitsvorschau. Der Empfänger benötigt HDR-Unterstützung. Original-Metadaten bleiben im Original erhalten, werden aber nicht vollständig in gerenderte Exporte übertragen.

**Kurven und Grading:** RGB-Gesamtkurve und einzelne Rot-/Grün-/Blau-Kurven mit jeweils fünf in X/Y bearbeitbaren Punkten; lineare Interpolation zwischen Punkten. Drei Farbräder für Tiefen/Mitteltöne/Lichter mit separater Helligkeit sowie Balance. Eigene Looks lassen sich unter Looks speichern, anwenden und löschen.

**Masken:** Zusätzlich Kontrast, Lichter, Tiefen, Weiß, Schwarz, Tönung, Struktur, Klarheit und Dunst entfernen. Farb- und Helligkeitsbereiche werden über Mittelpunkt/Toleranz als gespeicherte Maske erzeugt; anschließend mit Pinsel/Radierer korrigierbar. Diese Auswahl wird bei späteren Farbänderungen nicht automatisch neu berechnet.

**Reparieren/Klonen:** Unter Retusche aktivieren, Größe festlegen, zuerst die saubere Quelle und dann die Zielstelle antippen. „Helligkeit angleichen“ überträgt Struktur mit angeglichener lokaler Helligkeit; ausgeschaltet wird weich geklont. Mehrere Korrekturstellen sind möglich und über Undo rückgängig zu machen. Dies ist eine lokale Retusche, kein generatives Entfernen oder automatisches Auffüllen großer Objekte.

**Optik/Geometrie:** Begradigen und manuelle vertikale/horizontale Perspektive. An den Rändern können unvollständige Bildbereiche entstehen; danach zuschneiden. Automatische RAW-Objektivkorrektur erscheint nur, wenn der Decoder sie für das Foto unterstützt. Es gibt keine eigene umfassende Adobe-/Lensfun-Profilbibliothek. Beim Maskieren, Zuschneiden und Setzen von Retuschestellen wird die Geometriekorrektur in der Vorschau vorübergehend ausgesetzt, damit die Positionen auf dem Ausgangsbild stimmen; im Ergebnis und Export bleibt sie aktiv.

**Mediathek:** Alben zuordnen, nach Album und Mindestbewertung filtern, 0–5 Sterne vergeben und mehrere Projekte auswählen. „Aktuelle Bearbeitung übertragen“ kopiert Licht/Farbe/Kurven/Grading, bewahrt aber individuelle Masken, RAW-Weißabgleich, Zuschnitt und Reparaturstellen. Katalog und eigene Looks liegen lokal in `Studio-Katalog.json`; sie werden nicht automatisch über Geräte synchronisiert oder mit einem einzelnen Projekt exportiert. Jeder übertragene Bearbeitungsschritt ist im jeweiligen Projekt rückgängig zu machen.

**Grenzen:** Originale bis 128 MB; Arbeitsbild weiterhin maximal 50 MP und 8192 Pixel Seitenlänge. Größere Quellen werden für die Entwicklung verkleinert, das eingebettete Original bleibt unverändert. Die Original-Entwicklung verwendet eine Bildebene; Ebenenbefehle sind für diese neuen Projekte gesperrt, damit RAW-Daten nicht unbemerkt durch eine gerenderte Ebene ersetzt werden. Alte Ebenenprojekte funktionieren weiter. Eine unveränderte Desktop-App zeigt nur die Kompatibilitätsbasis und rendert die neuen iOS-Einstellungen nicht.

## iPad

Native Universal-App ab iOS/iPadOS 17, inklusive iPad Pro 12,9 Zoll. Die Oberfläche verwendet die vollständige Fensterfläche. Ab 760 Punkten Fensterbreite stehen die Werkzeuge rechts neben dem Bild; schmalere Fenster verwenden das untere Bedienfeld. Die Reglerfläche passt sich der verfügbaren Höhe und dem gewählten Werkzeug an. Alle vier iPad-Ausrichtungen sind freigegeben. Verkleinerte Fenster werden anhand ihrer tatsächlichen Größe angeordnet; mehrere gleichzeitige Dokumentfenster sind nicht implementiert.

Masken lassen sich mit Finger oder Apple Pencil zeichnen und korrigieren. **Nur Apple Pencil** im Maskenfeld ignoriert Fingerkontakte auf der Zeichenfläche. Die Pinselgröße wird über den Regler festgelegt; Druck, Neigung, Hover, Pencil-Doppeltipp und Pencil-Pro-Drücken steuern keine zusätzlichen Funktionen. Regler-Doppeltipp zum Zurücksetzen bleibt verfügbar.

iPhone und iPad speichern Projekte jeweils lokal. Für die Übertragung auf dem iPhone **Exportieren → PhotoCraft iOS-Projekt** wählen und die Datei etwa über AirDrop oder iCloud Drive übertragen. Auf dem iPad über **Mediathek → Dateien** öffnen. Es gibt keine automatische Synchronisierung der Projektmediatheken.

Installation über Xcode: dieselbe `PhotoCraft.xcodeproj` öffnen und das iPad als Ziel wählen. Am iPad dem Mac vertrauen und den Entwicklermodus aktivieren, falls iPadOS/Xcode dazu auffordert. Mit dem eigenen Personal Team signieren.

## Neu: automatische Maskierung

Unter **Aktionen** Motiv, Hintergrund, Himmel oder Objekte wählen. Unter **Retusche** Haut, Haar oder Augen wählen. Alternativ **Maskieren → Neue Maske → Automatisch**. Die Auswahl wird als eigene Maske angelegt; erkannte Vordergrundobjekte und Gesichtspartien mehrerer Personen werden getrennt angelegt (insgesamt höchstens zwölf Masken pro Foto). Nach der Erkennung Belichtung, Wärme und Sättigung getrennt einstellen. Die rote Überlagerung zeigt den ausgewählten Bereich. Pinsel und Radierer ergänzen oder entfernen Stellen; weiche Kante und Umkehren stehen ebenfalls zur Verfügung.

Die Erkennung läuft vollständig auf dem iPhone. Apple Vision trennt auffällige Vordergrundobjekte; das mitgelieferte DETR-Modell erkennt Himmel, Face Parsing erkennt Gesichtspartien. **Haut umfasst Gesicht, Ohren und sichtbaren Hals, keine vollständige Körperhaut-Erkennung.** Augen werden zusammen pro Person ausgewählt; Iris und Augenweiß sind keine separaten Auswahlen. Objekte meint erkannte Vordergrundinstanzen, keine beliebigen Objekte per Textbefehl. Kleine, verdeckte oder unscharfe Gesichter und feine Haare können Nacharbeit erfordern. Himmelkonturen an dünnen Ästen und Gebäuden prüfen. Die Ergebnisse entsprechen nicht pixelgenau Lightroom.

Masken werden im Projekt gespeichert und beim Zuschneiden, Drehen und Spiegeln mitgeführt. Ein Modell läuft nur bei der Maskenerstellung; beim Verschieben von Reglern wird die gespeicherte Maske verwendet. Die Modelle sind bereits enthalten und erfordern keinen separaten Download oder Cloud-Account. Herkunft und Lizenzen: `App/Models/README.md`.

## Bedienung und Werkzeuge

- **Foto öffnen:** über Mediathek → Foto öffnen oder Dateien. Jedes Foto wird ein eigenes Projekt; die Quelldatei bleibt erhalten.
- **Regler zurücksetzen:** direkt auf einen Schieberegler doppeltippen, um ihn auf **0** zu setzen. Der Reset wird gespeichert und lässt sich rückgängig machen. Für VoiceOver gibt es die Aktion „Auf null zurücksetzen“.
- **Licht:** Belichtung, Kontrast, Lichter, Tiefen, Weiß und Schwarz. **Kurve:** Tiefen, Mitteltöne und Lichter als drei veränderbare Stützpunkte.
- **Farbe:** Wärme, Tönung, Dynamik und Sättigung; Schwarzweiß und Weißabgleich-Pipette. Die Pipette auf eine neutrale graue oder weiße Stelle im Bild setzen. Sie arbeitet relativ in RGB; für RAW stehen zusätzlich die oben beschriebenen Kelvin-Regler bereit.
- **Farbmix:** Rot, Orange, Gelb, Grün, Aqua, Blau, Violett und Magenta jeweils mit Farbton, Sättigung und Luminanz.
- **Grading:** warme Lichter, kühle Tiefen und matte Schwarztöne. Zusätzlich die oben beschriebenen Dreiwege-Farbräder.
- **Effekte:** Struktur, Klarheit, Dunst entfernen, Unschärfe, Vignette mit Mittelpunkt und Körnung.
- **Details:** Schärfen mit Radius, Details und Kantenmaskierung; Luminanzrauschen mit Details/Kontrast; Farbrauschen mit Details/Glättung. Der Regler „Maskieren“ hier begrenzt die Schärfung auf Kanten und ist unabhängig von den lokalen Masken.
- **Optik:** manuelle Verzeichnung sowie getrennte rote und blaue Farbsäume. Die Regler müssen auf das jeweilige Foto abgestimmt werden. Bei unterstützten RAW-Dateien ist zusätzlich die Decoder-Objektivkorrektur verfügbar.
- **Masken:** Maskieren → Neue Maske → Pinsel, linearer Verlauf oder radialer Verlauf. Auf dem Foto malen bzw. ziehen. Pinselgröße, Radierer, weiche Kante, Umkehren, Aktivieren/Deaktivieren und Löschen über das Papierkorbsymbol oben im Maskenfeld. Pro Maske lokale Belichtung, Wärme, Sättigung und die zusätzlichen Build-9-Einstellungen. Bis zu zwölf Masken; die rote Überlagerung lässt sich mit dem Augensymbol ausblenden. Die Pinselüberlagerung erscheint während des Zeichnens, die Bildwirkung wird beim Loslassen übernommen.
- **Looks:** Natürlich, Golden, Nordic, Noir und Lebendig; zusätzlich unter **Looks → Cinema**: Teal & Amber, Bleach Bypass, Golden Film, Midnight, Desert, Soft Cinema, Neo Noir und Vintage 35. Über jedem Namen erscheint eine berechnete Miniatur des aktuellen Fotos mit dem jeweiligen Look und den vorhandenen Masken. Im Bedienfeld nach unten scrollen. Alle Looks setzen globale Entwicklungswerte und bleiben anschließend frei veränderbar; vorhandene lokale Masken bleiben erhalten.
- **Zuschnitt:** frei oder 1:1, 4:5, 16:9, dazu Drehen und Spiegeln. Maskenpositionen werden mitgeführt.
- **Ebenen:** auswählen, sichtbar/unsichtbar, duplizieren und löschen.
- **Vergleich:** direkt auf dem Foto gedrückt halten; nach kurzem Halten erscheint „VORHER“, beim Loslassen wieder die Bearbeitung. Alternativ die zwei Quadrate im Lichtfeld gedrückt halten. Bei aktiver Maskenzeichnung oder Zuschnitt zuerst das Bedienfeld ausblenden, damit die Bildgeste frei ist. Zeigt die Bildbasis vor den neuen iOS-Entwicklungseinstellungen einschließlich Masken. Frühere Desktop- oder Build-3-Einstellungsebenen gehören zur Bildbasis und bleiben erhalten.
- **Vollbild/Zoom:** den Griff oberhalb der Regler nach unten wischen. Am unteren Bildrand nach oben wischen oder den Griff antippen, um die Regler wieder zu öffnen. Alternativ oben Bedienelemente ausblenden; mit zwei Fingern zoomen, Doppeltipp setzt zurück.
- **Export:** oben rechts JPEG, PNG oder PhotoCraft-iOS-Projekt teilen. JPEG/PNG enthalten die neuen Effekte und Masken in Dokumentauflösung.

Die Reglerfelder sind vertikal scrollbar; die Werkzeugleiste ist horizontal scrollbar. Nach einer abgeschlossenen Änderung wird automatisch lokal gespeichert. Rückgängig oben links, Wiederholen im Exportmenü.

## Speicherung und Kompatibilität

Bildbasis und Ebenen bleiben im `.pcraft`-Projekt erhalten. Die neuen iOS-Entwicklungswerte und Masken werden nichtdestruktiv als versionierter Metadatensatz `photocraft.ios.development.v1` im selben Projekt gespeichert. Auch Undo/Redo erfasst diesen Satz. Es gibt keine separate, leicht verlierbare Sidecar-Datei.

**Die neuen iOS-Entwicklungseffekte werden beim Öffnen in der unveränderten Desktop-Version von PhotoCraft nicht gerendert.** Diese Version der iPhone-App kann sie wieder bearbeiten; für die Weitergabe des fertigen Bildlooks an andere Programme JPEG/PNG exportieren. Bestehende Build-3-Projekte bleiben lesbar. Deren alte Einstellungsebenen werden unverändert als Bildbasis übernommen; neue iOS-Regler starten bei neutralen Werten.

## Leistung und Grenzen

Die Bildbasis wird einmal geladen. Beim Ziehen werden die Einstellungen auf einer Vorschau bis 1600 Pixel Kantenlänge mit Core Image/Metal berechnet. Kein PNG-Encoding und kein Rust-Dokumentklonen pro Reglerbewegung. Fertige Zwischenbilder werden angezeigt, auch wenn bereits ein neuer Wert wartet; die Warteschlange hält nur den letzten noch nicht gestarteten Zustand. Reglerpositionen springen beim Ziehen nicht mehr.

Import bis 50 Megapixel und maximal 8192 Pixel pro Seite; Originaldateien bis 128 MB, Projektdateien bis 384 MB. Größere Bildquellen werden verkleinert. Der Export verwendet die Dokumentauflösung. Bis 24 oberste Ebenen; Undo maximal 20 Zustände und 256 MiB Pixelbudget. Große Importe, Exporte und das Speichern können weiterhin kurz dauern.

Die Filter sind eigene bzw. Apple-Core-Image-Implementierungen und liefern nicht pixelgenau die Adobe-Ergebnisse. Nicht enthalten sind generative Retusche, eine eigene vollständige Objektivprofilbibliothek, tiefenbasierte Porträtunschärfe und die vollständige Desktop-Werkzeugsammlung. Die Unschärfe ist ein regulärer Weichzeichner. Neue Original-/RAW- und Exportfunktionen sowie ihre Grenzen sind oben beschrieben.

## Privat auf dem iPhone installieren

1. `PhotoCraft.xcodeproj` in Xcode öffnen, Schema **PhotoCraft** wählen.
2. In **Xcode → Settings → Apple Accounts** den eigenen Apple-Account einrichten.
3. Unter **PhotoCraft → Signing & Capabilities** das eigene Personal Team wählen. Der lokale Projektstand ist für das bereits eingerichtete Team dieses Macs konfiguriert. Bei Weitergabe Team und Bundle-ID ändern.
4. iPhone anschließen, dem Mac vertrauen und den Entwicklermodus am iPhone aktivieren, falls Xcode dazu auffordert.
5. Das iPhone als Ziel wählen und **Run ▶** drücken.

Ein kostenloses Personal Team reicht für den privaten Gerätetest. Dessen Bereitstellungsprofile laufen nach sieben Tagen ab; die App muss dann erneut über Xcode gebaut und installiert werden. [Apple: Personal Team](https://developer.apple.com/help/account/basics/about-your-developer-account).

Rust über https://rustup.rs installieren, danach:

```sh
rustup toolchain install 1.95.0 --profile minimal
rustup target add --toolchain 1.95.0 aarch64-apple-ios aarch64-apple-ios-sim
```

Der erste optimierte Build des Bildbearbeitungskerns dauert mehrere Minuten. Die Folge-Builds verwenden den Cache. `PHOTOCRAFT_CARGO_TARGET_DIR` kann auf einen bereits vorhandenen Cargo-Buildordner zeigen. Für Intel-Simulatoren zusätzlich `x86_64-apple-ios` installieren.

## Tests, Quellcode und Herkunft

Prüfergebnisse: VALIDATION.md. Einstieg für andere Entwickler: WEITERENTWICKLUNG.md.

- `App/`: SwiftUI/UIKit, GPU-Renderer, Maskeneditor, Import/Export und Projektverwaltung.
- `Bridge/`: isolierte C-Anbindung, PhotoCraft-Projekte, Metadaten-History und Tests.
- `UITests/`: Bedienungs-, Import-, Masken- und Exporttests. Testbilder sind synthetisch.
- `vendor/photocraft/`: unveränderter Originalquellstand `8d840eab5962b41e4e15b115107eef8384648fb5`, https://github.com/storytold/photocraft. Ursprüngliche Lizenzen und Attributionen dort beibehalten.

Private Portierung, keine offizielle iOS-Veröffentlichung von PhotoCraft oder Adobe.

## Build 13 – Referenz-Look

Unter **Looks → Look aus Referenzfoto** ein Bild aus Fotos/Dateien oder einen direkten HTTPS-Bildlink laden. PicsRoom analysiert Helligkeitsverteilung, Kontrast, Farbsättigung und Farbbalance lokal und schlägt eine Annäherung über Belichtung, Kontrast, Sättigung, Wärme und Tönung vor. Die Stärke ist einstellbar; die Vorschau verändert das Projekt erst nach **Übernehmen**. Abbrechen verwirft den Vorschlag. Anschließend sind die Regler weiter editierbar und als eigenes Preset speicherbar. Rückgängig nutzt den bestehenden Projektverlauf.

Die Analyse rekonstruiert keine ursprünglichen Lightroom-Einstellungen und erkennt keine lokalen Masken der Vorlage. Unterschiedliche Motive beeinflussen das Ergebnis. RAW-Entwicklung, Masken, Geometrie und Schärfung bleiben erhalten. Es wird kein eigenes Foto hochgeladen und kein KI-Schlüssel benötigt. Links müssen direkt Bilddaten liefern; Webseiten, Social-Media-Seiten und Anmeldelinks werden mit einem Hinweis abgewiesen. Referenzen sind auf 32 MB begrenzt und werden für die Analyse verkleinert. Das Referenzfoto wird nicht im Projekt gespeichert.
