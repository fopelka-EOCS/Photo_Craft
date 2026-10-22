# Modelle für lokale automatische Masken

## Himmel

Apple DETR ResNet50 Semantic Segmentation F16, basierend auf facebook/detr-resnet-50-panoptic.
Quelle: https://huggingface.co/apple/coreml-detr-semantic-segmentation
Lizenz: Apache 2.0, siehe DETR-LICENSE.txt. Autoren: Nicolas Carion, Francisco Massa, Gabriel Synnaeve, Nicolas Usunier, Alexander Kirillov und Sergey Zagoruyko.
448 × 448 RGB-Eingabe, Ausgabe semanticPredictions. Die Himmelklasse wird aus den Modellmetadaten ermittelt. Das ganze Bild wird skaliert, nicht mittig abgeschnitten.

## Haut, Haare und Augen

Face Parsing / BiSeNet von zllrunning/face-parsing.PyTorch; Core-ML-Konvertierung von Daisuke Majima (john-rocky / mlboydaisuke).
Quelle: https://huggingface.co/mlboydaisuke/Face-Parsing-CoreML
Original: https://github.com/zllrunning/face-parsing.PyTorch
Lizenz: MIT, siehe FaceParsing-LICENSE.txt. Copyright (c) 2019 zll.
512 × 512 RGB-Eingabe, Ausgabe argmax_out. Pro erkanntem Gesicht wird ein erweiterter Porträtausschnitt verarbeitet. Haut: Klassen 1, 7, 8, 10, 14; Haare: 17; Augen: 4, 5.

## Motiv, Hintergrund und Objekte

Apple Vision VNGenerateForegroundInstanceMaskRequest. Kein externes Modell wird für diese Auswahlarten mitgeliefert.
https://developer.apple.com/documentation/vision/vngenerateforegroundinstancemaskrequest

Die heruntergeladenen Modelle sind unverändert. provenance.json enthält feste Quellrevisionen und SHA-256-Prüfsummen sämtlicher Paketdateien. Die App überträgt keine Fotos an diese Anbieter. Die Auswahlqualität ist motivabhängig; alle Masken lassen sich manuell ergänzen oder radieren.
