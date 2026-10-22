# Übergabe zur Weiterentwicklung

Stand: 17. Oktober 2026, Version 1.0, Build 12 (Zuschnitt, Formate und KI-Erweiterungsanbindung).

Der sichtbare App-Name ist PicsRoom. Die Xcode-Projektdatei und das Schema heißen zur Kompatibilität weiterhin PhotoCraft. Die bestehenden Projekt- und Bundle-Kennungen wurden bewusst beibehalten.

## In einem anderen Tool öffnen

Das ZIP entpacken und den gesamten Ordner `PicsRoom-iOS` als Projektordner öffnen, beispielsweise in einem Code-Editor oder einem anderen Coding-Assistenten. Alle eigens geschriebenen App- und Bridge-Quellen sowie der verwendete PhotoCraft-Originalquellcode sind enthalten. Eine Chat-Historie wird zum Bauen nicht benötigt.

- `App/CropTools.swift`: Zuschnittgeometrie, Formatsperre und acht Griffe.
- `App/AIExpansion.swift`: KI-Einrichtung, Keychain, Maskenvorbereitung, OpenAI-Aufruf und Ergebnisprüfung; Offline-Testtransport nur in DEBUG.
- `App/ExportView.swift`: Exportauswahl, Format/Größe/Qualität/Wasserzeichen und Speichern in Fotos.
- `App/EditorView.swift`: Oberfläche, Werkzeuge, Importdialoge und Projektübersicht.
- `App/Engine.swift`: Swift-Anbindung an den Rust-Kern, Import/Export, Speicherung und Vorschau.
- `App/DevelopRenderer.swift`: Core-Image/Metal-Pipeline für Vorschau und Vollauflösungsexport.
- `App/MaskEditor.swift`: Maskengesten, Weißabgleich-Pipette und dünne UIKit-Regler.
- `App/ZoomCanvas.swift`: Bildansicht und Gesten.
- `App/PhotoCraftApp.swift`: App-Einstieg.
- `Bridge/src/lib.rs`: C-Schnittstelle, Befehle, Entwicklungseinstellungen und Rust-Tests.
- `PhotoCraft.xcodeproj`: Xcode-Projekt und gemeinsames Schema PhotoCraft.
- `UITests/StudioTests.swift`: automatisierte Bedienungstests.
- `vendor/photocraft`: vollständiger verwendeter Originalquellstand mit Lizenzen.

## Bauen

Die Bearbeitung kann in einem beliebigen Editor erfolgen. Für einen iPhone-Build werden macOS und Xcode mit iOS-SDK benötigt. Geprüft wurde mit Xcode 27.0. Rust 1.95.0 und die passenden iOS-Ziele installieren; die Befehle stehen in README.md. Das Xcode-Buildskript baut den Rust-Kern automatisch. Cargo lädt die in den Lockdateien festgelegten externen Abhängigkeiten beim ersten Build aus dem Internet.

In Xcode das Projekt öffnen und das Schema PhotoCraft sowie ein iPhone oder einen Simulator wählen. Für ein physisches Gerät unter Signing & Capabilities das eigene Team wählen. Der Projektstand enthält die Team-ID des bisherigen lokalen Builds, aber keine privaten Signierschlüssel oder Apple-Zugangsdaten. Bei anderer Signierung gegebenenfalls auch die Bundle-ID ändern.

Das Buildskript erkennt den bisherigen lokalen Rust-Installationsordner, falls vorhanden; andernfalls verwendet es die normale Rust-Installation unter ~/.cargo/bin. Der bisherige work-Ordner ist nicht erforderlich.

## Aktueller Funktionsstand

Transparente Licht-/Farbregler, Live-Vorschau, Looks, Zuschnitt/Drehen/Spiegeln, einfache Ebenenverwaltung, Projektmediathek, automatische lokale Speicherung und PNG-/JPEG-/Projekt-Export. Die detaillierten Grenzen, insbesondere fehlende RAW-/HDR-Entwicklung und noch nicht portierte Desktop-Werkzeuge, stehen in README.md. Prüfresultate stehen in VALIDATION.md.

Build 3 behebt übergroße Berührungsflächen der Projektvorschaubilder sowie die Übergabe von der Projektübersicht an die Foto-/Dateiauswahl. Der Regressionstest testRepeatedPhotoImport benötigt das unter TestFixtures enthaltene Bild in der Simulator-Fotomediathek (vor dem Test per simctl addmedia laden).

## Umfang des Pakets

Enthalten: Quellcode, Bildressourcen, beide KI-Modelle mit Lizenzen und Herkunftsnachweis, Projektkonfiguration, Buildskript, Lockdateien, Tests und Dokumentation. Nicht enthalten: generierte Builds, Compiler-Caches, installierte Toolchains, persönliche Fotoprojekte, Zertifikate oder private Schlüssel. Externe Rust-Pakete werden über Cargo bezogen. Ursprüngliche Lizenz- und Attributionsdateien unter vendor/photocraft beibehalten.

## Build 4 – Architekturhinweise

NativeState (Development + LocalMask-Liste) liegt als JSON im Dokument-Metadatenfeld photocraft.ios.development.v1. Rust native speichert Änderungen über Session.edit; Undo/Redo und .pcraft-Speicherung bleiben im Kern. Bildbasis-Vorschauen kommen nur bei Öffnen und strukturellen Änderungen aus Rust. Entwickeln nutzt einen wiederverwendeten CIContext auf einer seriellen Render-Queue. PNG/JPEG exportieren zuerst die volle Basis und wenden anschließend dieselbe native Filterkette an. Desktop-PhotoCraft interpretiert den iOS-Metadatensatz nicht; für den fertigen Look ist ein Bildexport erforderlich.

--demo und --demo48 erzeugen synthetische Testbilder. --demo --verify-development prüft Filterwirkung und Masken-Roundtrip und schreibt development-verification.json in den App-Dokumentordner. --benchmark-only misst nach dem Öffnen des letzten Projekts ausschließlich Vorschauzeiten, ohne die Einstellungen zu verändern, und schreibt preview-benchmark.json. Normale Starts benötigen keine Argumente.

## Build 5

Maskenbindungen werden in EditorView über die Masken-ID aufgelöst; keine gespeicherten Array-Positionen in SwiftUI-Bindungen. CinemaLook in Engine.swift definiert acht neue, editierbare Presets. Die zusätzlichen UI-Tests heißen testDeleteMasksAndRestore und testCinemaLooks.

## Änderungen in Build 6

Look-Vorschauen werden auf der seriellen Render-Warteschlange mit maximal 256 Pixeln Kantenlänge berechnet und pro Bild-/Maskenrevision zwischengespeichert. Eine Revisionsprüfung verwirft veraltete Ergebnisse nach Bildwechseln. Die Vorschaubasis entspricht der für die Look-Anwendung verwendeten Bildbasis; aktuelle Masken bleiben dabei erhalten. Der Vergleich verwendet einen UIKit-Long-Press-Recognizer und setzt beim Loslassen oder Abbruch zurück. Der Griff am Bedienfeld lässt sich ziehen oder antippen. Der UI-Test `testLookThumbnailsAndPhotoGestures` prüft diese Bedienabläufe.

## Build 7

`AutomaticMasks.swift` kapselt Vision und die beiden mitgelieferten Core-ML-Modelle. Aufrufe laufen auf einer eigenen seriellen Warteschlange. Face Parsing verwendet auf dem Gerät CPU/GPU; der Neural-Engine-Pfad des älteren Modells lieferte im Gerätetest einen Vorhersagefehler. Gesichtserkennung im Simulator verwendet Revision 2 auf CPU. Modelle nur einzeln im Speicher halten.

`LocalMask` besitzt zwei optionale neue Felder: `rasterPNG` (Codable-Data als Base64) und `rasterCorners` (normierte obere linke, obere rechte und untere linke Ecke). Ältere Projekte bleiben lesbar. Die Rust-Bridge transformiert alle drei Eckpunkte bei Zuschnitt, Rotation und Spiegelung; PNG-Daten bleiben unverändert. Pinselstriche werden über die automatische Grundmaske gezeichnet. Ein begrenzter thread-sicherer Cache speichert fertige Masken getrennt von deren Belichtungs-/Farbwerten. Diese Daten gehören wie die bisherigen iOS-Entwicklungswerte zum iOS-spezifischen Metadatensatz.

Die UI-Tests brauchen für die KI-Tests `astronaut.png` und `rocket.jpg` im Dokumentordner der Simulator-App. Testquellen: scikit-image v0.22.0 (`skimage/data`, öffentliche NASA-Testfotos). Sie sind nicht Bestandteil der Produktions-App. Debug-Schalter `--verify-ai` prüft Modelle und Maskenpersistenz, `--portrait-demo` öffnet das Testporträt; `--verify-ai-device` prüft sieben Erkennungsarten ohne Öffnen oder Ändern eines Fotoprojekts. Diese Schalter sind aus Release-Builds ausgeschlossen.

## Build 8 – iPad

App und UI-Test-Target verwenden `TARGETED_DEVICE_FAMILY = "1,2"`. Die Info.plist enthält eine eigene iPad-Ausrichtungsliste mit allen vier Orientierungen. Die Fensterbreite bestimmt Seitenleiste oder unteres Bedienfeld; `StudioPanel` liest die verfügbare Höhe aus einer Environment-Eigenschaft. Canvas, Masken und Zuschnitt teilen dieselben Insets, sodass Bild und Gesten übereinstimmen.

Die Maskeneingabe nutzt eine transparente UIKit-Fläche mit genau einem aktiven Touch. Koaleszierte Pencil-/Finger-Samples werden übernommen, weitere Kontakte während eines Strichs ignoriert. Pencil-only filtert Touches nach `.pencil`. Bei Abbruch wird ein unvollständiger Pinselstrich verworfen bzw. die ursprüngliche Verlaufsgeometrie wiederhergestellt. Keine neue Dateiformatversion nötig.

Export verwendet eine Sheet-Präsentation auch auf dem iPad. Das Projekt bleibt identisch für iPhone und iPad; persönliche Fotoprojekte und Apple-Zugangsdaten sind nicht im Quellcode-Paket enthalten.


## Build 9: Originalbasierte Entwicklung

`AdvancedEditing.swift` definiert RAWSettings, SourceEdit, AdvancedSettings, Katalog und die Kurven-/Farbradansichten. Optionalfelder in NativeState und LocalMask erhalten die Lesbarkeit alter Projekte. `OriginalPipeline` erstellt einen unabhängigen CIRAWFilter pro Decode und wendet die gespeicherte Decoder-Version, Kelvin/Tint und Belichtung an. `DevelopRenderer.renderOriginal` hat einen begrenzten Halbfloat-Cache; voller Export wird getrennt berechnet. Die RAW-Vorschau ist SDR, der HLG-Export 10 Bit.

Die Bridge speichert Originalbytes als private PSD-Ressource ID 4000 mit Namensraum `photocraft.ios.source.v1:<UTI>` in der bestehenden inhaltsadressierten Blob-Speicherung des unveränderten Kerns. Das ist ein interner Ressourcenträger, kein PSD-Exportfeature. Arc vermeidet Kopien der Originalbytes in Undo-Zuständen. `attachSource` initialisiert Quellmetadaten ohne einen zusätzlichen Undo-Schritt; `extractSource` materialisiert das Original in einer temporären Datei. Snapshot überträgt nur den Typ, niemals die Originalbytes über JSON. `sourceEdits` enthält normalisierte Crop-/Rotate-/Flip-Schritte; diese werden zusammen mit Masken im Rust-Commandpfad und Undo erfasst. Ebenenbefehle für Originalprojekte werden explizit abgefangen, um keine gerenderte Ersatzquelle vorzutäuschen.

Katalog/Presets: atomare lokale JSON-Datei. Stapelbearbeitung ist seriell, bewahrt individuelle Geometrie/Masken/RAW-Einstellungen und öffnet anschließend das Ausgangsprojekt. Bei einem Fehler werden erfolgreich bearbeitete Projekte erhalten; die Meldung nennt die Anzahl. Keine Cloud-Synchronisation.

Die neuen UI-Tests `testRAWOriginalRoundtrip` benötigen `canon-250d.cr3` und `proraw.dng` im Simulator-Dokumentordner. Testdateien sind nicht im Quellarchiv enthalten. Quellen und Prüfstand: VALIDATION.md. `testAdvancedEditing` nutzt ein synthetisches Motiv; beide Prüfpfade sind nur in DEBUG verfügbar.

### Referenz-Look (Build 13)
`App/ReferenceLook.swift` enthält Import, begrenzten HTTPS-Download, ImageIO-Thumbnail-Decodierung, sRGB-Statistik und die Vorschauoberfläche. `EditorModel.referencePreview` verwendet dieselbe Renderpipeline wie der Editor, ohne den Projektzustand zu ändern. Übernommen wird eine Development-Kopie über `applyPreset`; vorhandene Masken und AdvancedSettings bleiben erhalten. Die neue Datei ist bereits im Xcode-Target enthalten. Der interne Projektname und Bundle-Identifier bleiben für bestehende Installationen unverändert.
