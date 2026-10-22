# Build 12 · Zuschnitt und KI-Erweitern · 17. Oktober 2026

- Zuschnittgeometrie für alle neun festen Verhältnisse, Hoch-/Querformat, acht Griffe und mehrere Ziehrichtungen geprüft: gültige Grenzen und konstantes Verhältnis.
- UI-Test erfolgreich: 4:5 wählen, Ecke ziehen, Verhältnis beibehalten, in Querformat wechseln, anwenden; Originalverhältnis nach Zuschnitt korrekt wieder auswählbar; KI-Einrichtung erreichbar und Start ohne Schlüssel/Einwilligung gesperrt.
- Export- und Scrollregression erfolgreich; der Bedienfeld-Kopfbereich ist reduziert.
- Presets, Mediathek-Auswahl und Maskenausblendung mit schmalem Bedienfeld erfolgreich nachgeprüft. Der Test scrollt präzise zum Regler; die frühere große Scrollgeste schoss im niedrigen Feld am Ziel vorbei.
- KI ausschließlich offline geprüft: PNG-Maske außen transparent/innen deckend, Request-Schema, festes HTTPS-Ziel, Zusammenfügen mit unverändertem Motiv in Ausgabeauflösung, HTTP 401 und ungültige Antwort.
- Kein echter OpenAI-Aufruf und keine Generierungsqualität bestätigt. Ein eigener eingerichteter API-Zugang ist erforderlich. API-Schlüssel wurden für die Tests weder ausgelesen noch gespeichert; der simulierte Transport verwendet nur einen Testwert.
- Signierter Release-Build erfolgreich; Build 12 auf iPhone 17 Pro installiert. Automatischer Start wegen Gerätesperre abgelehnt.

# Build 11 · Export und kompakter Editor · 16. Oktober 2026

- Simulator-Durchlauf: JPEG-Qualitätsstufen ergeben unterschiedliche Dateigrößen; Export auf 512 Pixel lange Kante geprüft; Wasserzeichen verändert die PNG-Ausgabe; zwei benannte Dateien exportiert und ursprüngliches Projekt wieder geöffnet; Originalexport byteidentisch; TIFF 16 Bit und HDR-HEIF 10 Bit geprüft.
- UI: niedrigeres Bedienfeld, Scrollen bis Schwarz-Regler und Ändern des Werts; Exportdialog und Größenwahl; Übergabe ans iOS-Teilen-Menü erfolgreich.
- Regression: Preset-Import, Mehrfachauswahl und Maskenausblendung mit kompakten Reglern erfolgreich.
- Direktes Speichern in Fotos einschließlich erteilter iOS-Hinzufügen-Berechtigung erfolgreich getestet. Ein früherer Test wählte versehentlich die Ablehnung; die Testauswahl wurde korrigiert und der Speichervorgang erneut erfolgreich geprüft.
- Signierter Release-Build erfolgreich; Build 11 auf verbundenem iPhone 17 Pro installiert und erfolgreich gestartet.

# Build 10 · PicsRoom · 16. Oktober 2026

- Signierter Release-Build erfolgreich. CFBundleDisplayName=PicsRoom, CFBundleVersion=10; Bundle-ID für vorhandene Daten beibehalten.
- Auf dem verbundenen iPhone 17 Pro installiert. Automatischer Start vom gesperrten Gerät abgelehnt; manuell nach Entsperren öffnen.
- XMP-Test mit Attributen und Elementwerten: drei unterstützte Einstellungen übernommen, nicht enthaltene Tiefen-Einstellung erhalten; JSON-Roundtrip und ungültiges XML geprüft.
- Mediathek: zwei Projekte auswählen, Fertig, zwischen den Projekten wechseln; Bearbeitungen bleiben erhalten.
- Fotoauswahl: zweimal jeweils zwei Fotos importiert und zum zweiten Foto gewechselt. UI-Test erfolgreich.
- Masken-UI-Test erfolgreich: sichtbaren Regler geändert, Überlagerung ausgeblendet, per Auge wieder eingeblendet. Die frühen fehlgeschlagenen Testläufe bewegten einen außerhalb des sichtbaren Panels liegenden Regler nicht; der Test prüft jetzt einen sichtbaren Regler.
- Lightroom-Kompatibilität ist bewusst begrenzt auf die im README genannten XMP-Felder; kein identisches Adobe-Rendering zugesichert.

Die früheren Prüfungen folgen als Historie.

# Prüfstand – 16. Oktober 2026, Build 9

- Acht Rust-Tests bestanden. Neu: Originalbytes in inhaltsadressierter Ressource, Speichern/Wiederöffnen/Extraktion bytegleich, Quellgeometrie und Undo. Bestehende Tests für Masken-Koordinaten, Importfehler und alte NativeState-Metadaten weiterhin erfolgreich.
- `testRAWOriginalRoundtrip` bestanden im iOS-27-Simulator: echte EOS-250D-CR3- und Apple-ProRAW-DNG-Datei importieren, RAW-Decoder statt JPEG-Vorschau erkennen, Kelvin/Belichtung verändern, sichtbare Pixeländerung prüfen, Projekt wieder öffnen, drehen und Undo, 16-Bit-TIFF exportieren und Farbtiefe auslesen, vollständiges Projekt teilen/importieren und Originalbytes vergleichen.
- `testAdvancedEditing` bestanden: RGB-Kurve, Grading, Perspektive/Begradigen, Klonen und Helligkeitsangleichung erzeugen geänderte Bilddaten. Grading-Persistenz, Bereichsmaske, eigene Looks, Album/Sterne-Persistenz, Stapelbearbeitung zweier Projekte mit Erhalt individueller Masken sowie 10-Bit-HEIF-Export geprüft.
- Regressionen `testDevelopmentAndMasks`, `testLookThumbnailsAndPhotoGestures`, `testDeleteMasksAndRestore` und `testRepeatedPhotoImport` bestanden. Letzter gemeinsamer Lauf: vier Tests ohne Fehler (RAW, erweiterte Bearbeitung, Maskenlöschung/Undo, wiederholter Fotoimport).
- Zusätzlicher Pixelvergleich bestanden: Eine Klonquelle außerhalb eines späteren Zuschnitts bleibt wirksam; normalisierte Quell-/Zielkoordinaten und Pinselradius werden zurück auf das Original abgebildet. RAW-Roundtrip nach dieser Änderung erneut bestanden.
- Der erste neue Bildtest fand einen ungültigen unendlichen Eingabebereich bei der Perspektivkorrektur; nach Korrektur bestand der Bildtest. Eine anfänglich zu genaue Textabfrage im UI-Test wurde auf den vollständigen Status im Header angepasst. Kurvenansicht visuell geprüft; Höhe danach an das Bedienfeld angepasst, damit die komplette Kurve sichtbar ist.
- Signierter Release-Build erfolgreich und auf dem angeschlossenen iPhone 17 Pro installiert. Der automatische Start wurde vom gesperrten iPhone abgelehnt. Die RAW-/HDR-Bildtests liefen im Simulator, nicht auf dem physischen iPhone. Kein Test eines eigenen iPhone-17-Pro-ProRAW oder einer eigenen Kameraaufnahme des Nutzers; keine allgemeine RAW-Formatgarantie.
- Grenzen: Editorvorschau SDR, HDR-Datei nicht auf einem kalibrierten HDR-Display beurteilt; Objektivkorrektur nur bei Decoder-Unterstützung. Retusche lokal statt generativ, Kurven mit fünf Punkten und linearer Interpolation, Alben/Looks lokal. Originalbasierte Projekte unterstützen keine separaten Ebenenbefehle. Details und weitere Grenzen: README.md.

Testdateien nur in der Arbeitsumgebung, nicht im Quellarchiv:
- EOS 250D: [Photography Blog – RAW-Beispiele](https://www.photographyblog.com/reviews/canon_eos_250d_review), Datei `canon_eos_250d_01.cr3` (nur lokaler Funktionstest, keine Weiterverteilung).
- Apple ProRAW: [raw.pixls.us, Datei 4264](https://raw.pixls.us/getfile.php/4264/nice/Apple%20-%20iPhone%2012%20Pro%20-%208bit%20(4%3A3).DNG), dort CC0; iPhone 12 Pro. Unterstützungsreferenz: [Apple RAW-Kameraliste iOS 27](https://support.apple.com/en-us/127493).

# Prüfstand – 16. Oktober 2026, Build 8

- Universal-App für iPhone und iPad; iPad-Ausrichtungen inklusive Hochformat, Querformat und umgekehrtem Hochformat freigegeben. Werkzeuge ab 760 Punkten Fensterbreite seitlich, darunter unten.
- `testIPadWorkspaceAndExport` auf iPad Pro 11 Zoll bestanden: Hoch-/Querformat, sichtbare und erreichbare Regler, Bedienfeld innerhalb des Fensters, Fingerzeichnung, Fingerkontakte bei „Nur Apple Pencil“ ignoriert, Maske löschen und PNG-Teilen ohne Absturz.
- Darstellung auf einem iPad Pro 12,9 Zoll (6. Generation) im Simulator visuell geprüft; zusätzlich Querformat des 11-Zoll-iPads geprüft.
- Pencil-Eingabe über native UIKit-Berührungen einschließlich gebündelter Bewegungspunkte. Keine Prüfung mit physischem Apple Pencil; keine Druck-/Neigungssteuerung. Stage Manager und interaktives Ändern der Fenstergröße wurden nicht separat am Gerät getestet.
- iPhone-Regression: `testDevelopmentAndMasks` und `testLookThumbnailsAndPhotoGestures` bestanden; manuelle Masken, Look-Vorschauen, Vorher/Nachher und Aus-/Einblenden der Regler geprüft.
- Signierter Release-Build erfolgreich. Buildnummer 8 und beide Gerätefamilien (iPhone/iPad) in der gebauten App geprüft.
- Installation auf dem angeschlossenen iPad noch ausstehend: Gerätekopplung scheitert mit `kAMDMCProtected`; Entsperren und Vertrauensbestätigung am iPad erforderlich. Die folgenden Build-7-Gerätetests gelten für das iPhone, nicht für das iPad.

# Prüfstand – 16. Oktober 2026, Build 7

- Doppeltipp auf globale und lokale Schieberegler setzt auf null; globale Undo-Wiederherstellung geprüft. VoiceOver bietet eine explizite Reset-Aktion.
- Neue zweistufige Werkzeugleiste mit blauer Auswahl, größeren Symbolen, Aktionen und Retusche. Screenshots der Bearbeitung und automatischen Haarmaske visuell geprüft.
- Gerätetest auf dem angeschlossenen iPhone 17 Pro: alle sieben Auswahlarten Motiv, Objekte, Hintergrund, Himmel, Haut, Haar und Augen erzeugten jeweils eine Maske auf öffentlichen Testfotos. Die Tests übertragen keine persönlichen Fotos. Haut, Haare, Augen und Himmel als rote Überlagerungen visuell geprüft. Dies ist ein Funktionstest mit zwei Motiven, keine allgemeine Genauigkeitsmessung.
- Das ältere Face-Parsing-Modell scheiterte zunächst auf dem Neural-Engine-Pfad. Die finale Fassung verwendet dafür CPU/GPU und besteht den Gerätetest. Apple Vision zur Gesichtserkennung benötigt im Simulator den CPU-Pfad mit Revision 2.
- `testAutomaticMasks` bestanden: vier Modellbereiche im Simulator, PNG-/JSON-Roundtrip, Rendern, Projekt speichern und wieder öffnen, Maskenrotation und Undo sowie Löschen und Wiederherstellen.
- `testPortraitMaskFromToolbar` bestanden: Testporträt öffnen, Retusche → Haar, automatische Auswahl erzeugen, Überlagerung anzeigen, lokale Belichtung verändern und per Doppeltipp zurücksetzen.
- `testSliderDoubleTapReset` bestanden: globale Belichtung zurücksetzen, Undo und neue Retusche-Auswahl. Der erste Testlauf verwendete einen Dezimalpunkt statt des deutschen Dezimalkommas; die Abfrage ist korrigiert.
- Regressionstest für manuelle Maskenlöschung und Undo bestanden. Sieben Rust-Tests bestanden, darunter Rastermasken-Koordinaten nach Zuschnitt, Drehen und Spiegeln sowie unveränderte PNG-Daten und Undo.
- Abschließender Regressionstest `testLookThumbnailsAndPhotoGestures` bestanden: echte Look-Vorschauen, Gedrückthalten für Vorher/Nachher und Regler aus-/einblenden nach der neuen Werkzeugaufteilung.
- Finale Release-App erfolgreich gebaut, auf dem iPhone installiert und gestartet. Die Bedienungstests liefen im Simulator; die sieben Erkennungsarten zusätzlich auf dem physischen Gerät.
- Grenzen: Gesicht-/Hals-Haut statt vollständiger Körperhaut, beide Augen pro Person gemeinsam, keine Iris-/Augenweiß-Trennung; nur auffällige Vordergrundobjekte. Segmentierungsauflösung 448 × 448 (Himmel), 512 × 512 je Porträtausschnitt, bis 1024 Pixel Kantenlänge für gespeicherte Vordergrundmasken. Kanten bei Haaren, dünnen Strukturen und kleinen Gesichtern können manuelle Korrekturen erfordern.

# Prüfstand – 15. Oktober 2026, Build 6

- Echte Fotominiaturen für alle fünf Standard-Looks und acht Cinema-Looks; Berechnung aus der aktuellen Bildbasis einschließlich vorhandener Masken. Asynchrone Berechnung mit Cache und Schutz gegen veraltete Ergebnisse nach Bildwechseln.
- Bedienungstest `testLookThumbnailsAndPhotoGestures` bestanden: Vergleich durch Gedrückthalten und Rückkehr beim Loslassen, Regler nach unten ausblenden und nach oben wieder öffnen, berechnete Look-Miniaturen vorhanden. Screenshots für ausgeblendete Regler und Cinema-Fotovorschauen visuell geprüft.
- Im ersten Lauf fehlte die DEBUG-Kompilierbedingung für die interne Vergleichs-Testmeldung. Nach Korrektur der Debug-Konfiguration bestand der vollständige Gesten-/Vorschautest. Der Release-Build enthält diese Testmeldung nicht.
- Regressionstest für Maskenlöschung, Undo und Wiederöffnen erneut bestanden.
- Signierter Release-Build erfolgreich; Build 6 auf dem angeschlossenen iPhone installiert und erfolgreich gestartet. Die automatisierten Gestentests liefen im Simulator; eine vollständige manuelle Bedienungsprüfung auf dem physischen Gerät steht aus.

# Prüfstand – 15. Oktober 2026, Build 5

- Maskenbindungen in Reglern, Schaltern und Zeichenfläche verwenden stabile IDs statt gespeicherter Array-Indizes. Nach dem Löschen ignorieren verbliebene Bindungen Schreibzugriffe; Lesezugriffe erhalten den letzten sicheren Wert. Dadurch greifen sie weder außerhalb der Liste noch auf eine andere Maske zu.
- Beim Wiederherstellen des Dokuments wird eine gültige Maskenauswahl hergestellt.
- Neuer Bedienungstest bestanden: drei Masken anlegen, mittlere Maske löschen, verbleibende Masken bis zur letzten löschen, Undo, erneut löschen, App neu starten und gespeicherten Zustand prüfen.
- Neuer Cinema-Test bestanden: alle acht Looks erzeugen unterschiedliche Bilddaten gegenüber dem neutralen Bild; Look über die Oberfläche anwenden und Erhalt vorhandener Masken prüfen.
- Simulator-Screenshot der Cinema-Auswahl geprüft. Signierter Release-Build erfolgreich. Build 5 auf dem angeschlossenen iPhone installiert und erfolgreich gestartet.

# Prüfstand – 15. Oktober 2026, Build 4

- Wartende Entwicklungseinstellungen während eines laufenden Speichervorgangs werden anschließend übernommen; In-App-Prüfung und Masken-Bedienungstest bestanden.
- Sechs Rust-Tests bestanden. Neu: native Entwicklungseinstellungen und Masken, Speicherung/Wiederöffnen, Koordinaten nach Zuschnitt und Undo.
- Fünf Bedienungstests erfolgreich geprüft: neue Entwicklung/Masken/Farbmix/Pinselgeste, 48-MP-Import und JPEG-Export, Projektmediathek/Querformat, wiederholter Fotoimport, Bearbeitung/Zuschnitt/PNG-Export. Der erste Gesamtlauf hatte einen Fehler im Test beim Abfragen eines noch außerhalb der sichtbaren Werkzeugleiste liegenden Buttons. Nach Anpassung des Test-Scrollens bestanden die betroffenen Bearbeitungs- und Maskentests erneut.
- In-App-Bildprüfung: Belichtung und neunzehn neue Filter sowie Farbmix und alle drei Maskentypen erzeugen veränderte Bilddaten; Maskeneinstellungen bleiben nach erneutem Öffnen erhalten. GPU-Kernel wurden initialisiert und auf dem Gerät verwendet.
- Numerischer Lauf auf dem angeschlossenen iPhone 17 Pro: 20 Vorschau-Berechnungen nach Aufwärmen, 900 × 1600 Pixel, Mittelwert 2,351 ms; Einzelwerte 2,020–5,187 ms. Dies misst Berechnung einschließlich Rückgabe des Vorschaubildes, nicht Eingabe-zu-Bildschirm-Latenz oder garantierte FPS. Komplexe Filter/mehrere Masken können länger dauern.
- Simulator-Messungen bei 1200 × 1600 Pixeln schwankten je nach gleichzeitiger Buildlast; kein pauschaler Beschleunigungsfaktor wird daraus abgeleitet.
- Screenshots von Licht, Farbe, Maskierung und Zuschnitt geprüft.
- Release-Build erfolgreich; finale App als Update auf dem iPhone installiert. Die vorherige Build-4-Fassung wurde auf dem Gerät gestartet und gemessen. Der Startversuch nach dem letzten Update wurde vom gesperrten iPhone abgelehnt; nach Entsperren die App manuell öffnen. Für die Geschwindigkeitsmessung wurden keine persönlichen Bilder kopiert, nur die numerische Ergebnisdatei.
- Eine vollständige manuelle Durchprüfung aller Werkzeuge auf dem physischen iPhone ist noch nicht erfolgt. Verfügbare Funktionen, Projektkompatibilität und Grenzen stehen in README.md.

## Frühere Prüfstände

# Prüfstand – 14. Oktober 2026

- Fünf Rust-Tests bestanden: Fehlerbehandlung, Bearbeitung/Undo, Einstellungsebenen, Projekt speichern/wieder öffnen, Zuschnitt/PNG-Export, Ebenengrenze sowie unverändernde Live-Vorschau und ein Undo-Schritt für eine komplette Anpassung.
- Swift-Typprüfung gegen das iOS-Simulator-SDK 27.0 bestanden.
- Optimierte Rust-Bibliotheken für ARM64-iOS und ARM64-iOS-Simulator gebaut.
- Vollständiger Simulator-Build und alle drei automatischen Bedienungstests bestanden: 48-MP-Import/Export, Mediathek/Querformat sowie Regler/Zuschnitt/Export.
- Export eines synthetischen 48-MP-Motivs als JPEG mit 6048 × 8064 Pixeln geprüft. Dieser Auflösungstest erfolgte im Simulator, nicht mit einem persönlichen Foto auf dem Gerät.
- Screenshots von Hochformat, transparenten Farbreglern, freiem Zuschnitt und Querformat visuell geprüft.
- Finaler Release-Build für das angeschlossene iPhone 17 Pro mit dem eingerichteten Personal Team erfolgreich signiert.
- App auf dem iPhone installiert; der anschließende Start wurde vom Gerät erfolgreich bestätigt. Eine vollständige manuelle Bedienungsprüfung auf dem physischen Gerät steht noch aus.

Die verfügbaren Funktionen und verbleibenden Grenzen sind in README.md beschrieben. Diese Fassung enthält noch nicht sämtliche Desktop-Werkzeuge oder RAW-/HDR-Entwicklung.

## Importkorrektur – Build 3

- Foto- und Dateiimport aus der Projektübersicht warten auf deren vollständiges Schließen.
- Berührungsflächen der Projektvorschaubilder auf die sichtbaren Karten begrenzt, damit sie keine Importbuttons überdecken.
- Neuer Regressionstest `testRepeatedPhotoImport` bestanden: aus einem geöffneten Dokument heraus zweimal nacheinander ein Foto über die System-Fotoauswahl importieren; Dokumentmaße und verfügbare Bedienelemente geprüft. Der Test nutzt `PhotoCraft-iPhone.png` (1206 × 2622), vorher über `simctl addmedia` in die Simulator-Fotomediathek geladen.
- Release-Build für das iPhone erfolgreich.
- Build 3 als Update auf dem angeschlossenen iPhone installiert.

## Build 13 – 2026-10-21
- Simulator-Debug-Build und signierter iPhone-Release-Build: erfolgreich.
- `testReferenceLook`: bestanden. Öffnen, leere Übernahme gesperrt, synthetische Bildreferenz laden/analysieren, Vorschau, Stärke verändern, Übernahme und erneutes Öffnen/Abbrechen.
- DEBUG-Prüfungen im selben Test: identische Statistik erzeugt keine Änderung; Stärke null erhält alle Ausgangswerte; abweichende Referenz ändert Farb-/Lichtwerte; Details/HSL bleiben erhalten; ungültige Bilddaten werden abgewiesen.
- Build 13 auf dem iPhone installiert. Automatischer Start wegen gesperrtem Gerät nicht möglich. Aktualisiertes Projekt in Xcode geöffnet.
- Nicht in diesem Durchlauf geprüft: externer HTTPS-Bilddownload gegen einen echten Anbieter, Fotomediathek-/Dateiauswahl der neuen Oberfläche und ästhetische Übereinstimmung bei beliebigen realen Referenzmotiven. Die Übernahme ist eine statistische Annäherung, keine Rekonstruktion fremder Bearbeitungsparameter.

## Public PR packaging — 2026-10-22
- Prepared against upstream main `78b46b1a`; the isolated bridge still uses engine pin `8d840eab5962b41e4e15b115107eef8384648fb5`.
- Preparation script: extracted engine from Git history and verified all existing model files against the pinned SHA-256 manifest. Fresh network model downloads were not exercised in this check.
- Personal signing team removed; project syntax and staged-file hygiene checks passed.
- After rustfmt and three Clippy structural fixes: all 8 bridge tests passed; `cargo clippy --all-targets -- -D warnings` passed.
- Final unsigned Debug build for the concrete arm64 iPhone simulator passed.
- Generic dual-architecture simulator build failed at the x86_64 link: the current script emits a single-architecture Rust archive. Select a concrete arm64 simulator; universal simulator packaging remains open.
- Full upstream desktop/web CI, layer checks, current-main engine compatibility and physical installation of this packaging copy were not verified. Historical standalone app tests above are separate evidence.
