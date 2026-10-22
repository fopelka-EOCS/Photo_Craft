import XCTest

final class StudioTests: XCTestCase {
    @MainActor private func selectTool(_ name:String, in app:XCUIApplication) {
        if name == "Masken" { app.buttons["tool-Masken"].tap();return }
        if ["Looks","Zuschnitt"].contains(name) { app.buttons["section-"+name].tap();return }
        app.buttons["section-Bearbeiten"].tap()
        let button=app.buttons["tool-"+name]
        let dock=app.scrollViews["tool-dock"]
        for _ in 0..<3 { dock.swipeRight() }
        for _ in 0..<6 {
            let frame=button.frame
            if frame.width>10 && frame.minX>=0 && frame.maxX<=app.windows.firstMatch.frame.width { break }
            dock.swipeLeft()
        }
        button.tap()
    }
    @MainActor private func reveal(_ element:XCUIElement,in app:XCUIApplication) {
        let panel=app.scrollViews["settings-panel"]
        for _ in 0..<12 {
            let frame=element.frame,viewport=panel.frame
            if frame.midY>viewport.minY+18 && frame.midY<viewport.maxY-18 && element.isHittable {return}
            let distance=max(-45,min(45,viewport.midY-frame.midY))
            let start=panel.coordinate(withNormalizedOffset:CGVector(dx:0.02,dy:0.5))
            start.press(forDuration:0.1,thenDragTo:start.withOffset(CGVector(dx:0,dy:distance)),withVelocity:.slow,thenHoldForDuration:0.2)
        }
    }
    @MainActor private func waitForSave(_ app:XCUIApplication) {
        let library=app.buttons["Mediathek"]
        XCTAssertTrue(library.waitForExistence(timeout:60))
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:library)
        waitForExpectations(timeout:60)
    }
    @MainActor func testReferenceLook() throws {
        let app=XCUIApplication();app.launchArguments=["--demo","--test-reference"];app.launch()
        waitForSave(app);selectTool("Looks",in:app)
        let open=app.buttons["reference-look"];reveal(open,in:app);open.tap()
        XCTAssertTrue(app.buttons["reference-apply"].waitForExistence(timeout:10))
        XCTAssertFalse(app.buttons["reference-apply"].isEnabled)
        let fixture=app.buttons["reference-fixture"]
        XCTAssertTrue(fixture.waitForExistence(timeout:10))
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:app.buttons["Fotos"])
        waitForExpectations(timeout:30)
        fixture.tap()
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:app.buttons["reference-apply"])
        waitForExpectations(timeout:30)
        XCTAssertTrue(app.sliders["reference-strength"].exists)
        app.sliders["reference-strength"].adjust(toNormalizedSliderPosition:0.5)
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:app.buttons["reference-apply"])
        waitForExpectations(timeout:30)
        app.buttons["reference-apply"].tap()
        XCTAssertTrue(open.waitForExistence(timeout:30));waitForSave(app)
        open.tap();app.buttons["Abbrechen"].tap();XCTAssertTrue(open.waitForExistence(timeout:10))
    }
    @MainActor func testCropRatiosAndExpansionSetup() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-crop-ai"];app.launch()
        let done=app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Zuschnitt und KI-Anbindung geprüft")).firstMatch
        expectation(for:NSPredicate {_,_ in done.exists || app.alerts.firstMatch.exists},evaluatedWith:app);waitForExpectations(timeout:120)
        XCTAssertTrue(done.exists,app.debugDescription)
        let panel=app.scrollViews["settings-panel"]
        XCTAssertLessThan(panel.frame.height,170)
        let initial=XCTAttachment(screenshot:XCUIScreen.main.screenshot());initial.name="compact-header-build12";initial.lifetime = .keepAlways;add(initial)
        selectTool("Zuschnitt",in:app)
        app.buttons["crop-ratios"].tap();app.buttons["4:5"].tap()
        let overlay=app.otherElements["crop-overlay"]
        XCTAssertEqual(overlay.value as? String,"0.8000")
        let handle=app.descendants(matching:.any)["crop-handle-3"]
        let start=handle.coordinate(withNormalizedOffset:CGVector(dx:0.5,dy:0.5))
        start.press(forDuration:0.1,thenDragTo:start.withOffset(CGVector(dx:-40,dy:-40)))
        XCTAssertEqual(overlay.value as? String,"0.8000")
        let orientation=app.buttons["Hochformat / Querformat"];reveal(orientation,in:app);orientation.tap()
        XCTAssertEqual(overlay.value as? String,"1.2500")
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="crop-locked-ratio";shot.lifetime = .keepAlways;add(shot)
        reveal(app.buttons["Anwenden"],in:app);app.buttons["Anwenden"].tap();waitForSave(app)
        selectTool("Zuschnitt",in:app);app.buttons["Original"].tap()
        XCTAssertEqual(overlay.value as? String,"0.7500")
        app.buttons["crop-ratios"].tap();app.buttons["9:16"].tap()
        let horizontal=app.scrollViews["crop-formats"]
        horizontal.swipeLeft()
        let expand=app.buttons["KI-Erweitern"]
        XCTAssertTrue(expand.waitForExistence(timeout:10));expand.tap()
        let startAI=app.buttons["ai-expand-start"]
        app.swipeUp()
        XCTAssertTrue(startAI.waitForExistence(timeout:20));XCTAssertFalse(startAI.isEnabled)
        XCTAssertTrue(app.secureTextFields["Persönlicher API-Schlüssel"].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testSaveExportToPhotos() throws {
        let app=XCUIApplication();app.launchArguments=["--demo"];app.launch();waitForSave(app)
        app.buttons["Exportieren und speichern"].tap();app.swipeUp()
        let button=app.buttons["export-photos"]
        for _ in 0..<3 {if button.isHittable {break};app.swipeUp()}
        button.tap()
        let springboard=XCUIApplication(bundleIdentifier:"com.apple.springboard")
        let alert=springboard.alerts.firstMatch
        if alert.waitForExistence(timeout:5) {
            let allow=alert.buttons.matching(NSPredicate(format:"(label CONTAINS[c] 'Allow' OR label CONTAINS[c] 'Erlauben' OR label CONTAINS[c] 'Hinzufügen') AND NOT (label CONTAINS[c] 'Don' OR label CONTAINS[c] 'Nicht')")).firstMatch
            XCTAssertTrue(allow.exists,springboard.debugDescription);allow.tap()
        }
        XCTAssertTrue(app.staticTexts["1 Foto(s) in Fotos gespeichert."].waitForExistence(timeout:60),app.debugDescription)
    }
    @MainActor func testExportOptionsAndCompactEditor() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-export"];app.launch()
        let done=app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Exportoptionen geprüft")).firstMatch
        expectation(for:NSPredicate { _,_ in done.exists || app.alerts.firstMatch.exists },evaluatedWith:app);waitForExpectations(timeout:180)
        XCTAssertTrue(done.exists,app.debugDescription)
        let panel=app.scrollViews["settings-panel"]
        XCTAssertLessThan(panel.frame.height,220)
        let initial=XCTAttachment(screenshot:XCUIScreen.main.screenshot());initial.name="compact-editor";initial.lifetime = .keepAlways;add(initial)
        let blacks=app.sliders["slider-blacks"]
        for _ in 0..<5 {
            if blacks.frame.midY>panel.frame.minY && blacks.frame.midY<panel.frame.maxY-10 {break}
            let start=panel.coordinate(withNormalizedOffset:CGVector(dx:0.02,dy:0.85))
            start.press(forDuration:0.05,thenDragTo:panel.coordinate(withNormalizedOffset:CGVector(dx:0.02,dy:0.2)))
        }
        XCTAssertTrue(blacks.isHittable,app.debugDescription)
        XCTAssertLessThan(blacks.frame.midY,panel.frame.maxY)
        blacks.adjust(toNormalizedSliderPosition:0.65);waitForSave(app)
        app.buttons["Exportieren und speichern"].tap()
        XCTAssertTrue(app.buttons["export-format"].waitForExistence(timeout:10))
        let export=XCTAttachment(screenshot:XCUIScreen.main.screenshot());export.name="export-options";export.lifetime = .keepAlways;add(export)
        app.buttons["export-size"].tap();app.buttons["2048 px · lange Kante"].tap()
        app.swipeUp()
        let share=app.buttons["export-share"]
        for _ in 0..<3 {if share.isHittable {break};app.swipeUp()}
        share.tap()
        XCTAssertTrue(app.otherElements["ActivityListView"].waitForExistence(timeout:60) || app.buttons["Copy"].exists || app.buttons["Kopieren"].exists,app.debugDescription)
    }
    @MainActor func testPresetsAndSelection() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-presets"];app.launch()
        let done=app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Presets und Auswahl geprüft")).firstMatch
        expectation(for:NSPredicate { _,_ in done.exists || app.alerts.firstMatch.exists },evaluatedWith:app);waitForExpectations(timeout:180)
        XCTAssertTrue(done.exists,app.debugDescription)
        XCTAssertTrue(app.buttons["Nächstes Foto"].exists)
        app.buttons["Mediathek"].tap()
        let choices=app.switches.matching(identifier:"Auswählen")
        XCTAssertGreaterThanOrEqual(choices.count,2)
        choices.element(boundBy:0).tap();choices.element(boundBy:1).tap()
        app.buttons["Fertig"].tap();waitForSave(app)
        XCTAssertTrue(app.buttons["Nächstes Foto"].isEnabled)
        app.buttons["Nächstes Foto"].tap();waitForSave(app)
        XCTAssertTrue(app.buttons["Vorheriges Foto"].isEnabled)
        selectTool("Masken",in:app);app.buttons["new-mask"].tap();app.buttons["Radialer Verlauf"].tap();waitForSave(app)
        let eye=app.buttons["mask-overlay"]
        XCTAssertEqual(eye.value as? String,"Sichtbar")
        let slider=app.sliders["mask-Weiche Kante"]
        reveal(slider,in:app)
        XCTAssertTrue(slider.isHittable,app.debugDescription)
        slider.adjust(toNormalizedSliderPosition:0.8)
        XCTAssertNotEqual(slider.value as? String,"60 %")
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="mask-after-slider";shot.lifetime = .keepAlways;add(shot)
        XCTAssertEqual(eye.value as? String,"Ausgeblendet")
        waitForSave(app);reveal(eye,in:app);eye.tap();XCTAssertEqual(eye.value as? String,"Sichtbar")
    }
    @MainActor func testRAWOriginalRoundtrip() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-raw"];app.launch()
        let done=app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","RAW geprüft: 2 Originale")).firstMatch
        let ready=NSPredicate { _,_ in done.exists || app.alerts.firstMatch.exists }
        expectation(for:ready,evaluatedWith:app);waitForExpectations(timeout:240)
        XCTAssertTrue(done.exists,app.debugDescription)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testAdvancedEditing() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-advanced"];app.launch()
        let done=app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Erweiterungen geprüft")).firstMatch
        let ready=NSPredicate { _,_ in done.exists || app.alerts.firstMatch.exists }
        expectation(for:ready,evaluatedWith:app);waitForExpectations(timeout:180)
        XCTAssertTrue(done.exists,app.debugDescription)
        selectTool("Kurve",in:app)
        XCTAssertTrue(app.buttons["Rot"].exists)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="rgb-curve";shot.lifetime = .keepAlways;add(shot)
        app.buttons["section-Retusche"].tap()
        XCTAssertTrue(app.switches["Reparieren / Klonen"].exists)
    }
    @MainActor func testIPadWorkspaceAndExport() throws {
        let app=XCUIApplication();app.launchArguments=["--demo","--verify-gestures"];app.launch();waitForSave(app)
        defer { XCUIDevice.shared.orientation = .portrait }
        XCTAssertGreaterThan(app.windows.firstMatch.frame.width,700)
        for orientation in [UIDeviceOrientation.portrait,.landscapeLeft] {
            XCUIDevice.shared.orientation=orientation
            let panel=app.scrollViews["settings-panel"]
            XCTAssertTrue(panel.waitForExistence(timeout:15))
            XCTAssertTrue(app.sliders["slider-exposure"].isHittable)
            let screen=app.windows.firstMatch.frame
            XCTAssertGreaterThan(panel.frame.minX,screen.width*0.45)
            XCTAssertLessThanOrEqual(panel.frame.maxX,screen.width)
            XCTAssertLessThanOrEqual(panel.frame.maxY,screen.height)
            XCTAssertGreaterThan(panel.frame.height,300)
            let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name=orientation == .portrait ? "ipad-portrait" : "ipad-landscape";shot.lifetime = .keepAlways;add(shot)
        }
        selectTool("Masken",in:app);app.buttons["new-mask"].tap();app.buttons["Pinsel"].tap();waitForSave(app)
        let pencil=app.switches["pencil-only"]
        XCTAssertTrue(pencil.waitForExistence(timeout:10));pencil.tap()
        let point=app.coordinate(withNormalizedOffset:CGVector(dx:0.28,dy:0.45))
        let end=point.withOffset(CGVector(dx:60,dy:40))
        point.press(forDuration:0.1,thenDragTo:end)
        XCTAssertEqual(app.otherElements["mask-input"].value as? String,"0")
        pencil.tap();point.press(forDuration:0.1,thenDragTo:end);waitForSave(app)
        XCTAssertEqual(app.otherElements["mask-input"].value as? String,"1")
        XCTAssertFalse(app.alerts.firstMatch.exists)
        app.buttons["Maske löschen"].tap();waitForSave(app)
        app.buttons["Exportieren und speichern"].tap();app.buttons["export-format"].tap();app.buttons["PNG"].tap();app.swipeUp();app.buttons["export-share"].tap()
        XCTAssertTrue(app.otherElements["ActivityListView"].waitForExistence(timeout:60) || app.buttons["Copy"].exists || app.buttons["Kopieren"].exists,app.debugDescription)
        XCTAssertEqual(app.state,.runningForeground)
    }
    @MainActor func testSliderDoubleTapReset() throws {
        let app=XCUIApplication();app.launchArguments=["--demo"];app.launch();waitForSave(app)
        let slider=app.sliders["slider-exposure"]
        slider.adjust(toNormalizedSliderPosition:0.7);waitForSave(app)
        slider.doubleTap();waitForSave(app)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label MATCHES %@","0[.,]00 EV")).firstMatch.exists,app.debugDescription)
        app.buttons["Rückgängig"].tap();waitForSave(app)
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format:"label MATCHES %@","0[.,]00 EV")).firstMatch.exists)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="new-editing-design";shot.lifetime = .keepAlways;add(shot)
        app.buttons["section-Retusche"].tap()
        XCTAssertTrue(app.buttons["auto-Haut"].exists)
        XCTAssertTrue(app.buttons["auto-Haar"].exists)
        XCTAssertTrue(app.buttons["auto-Augen"].exists)
    }
    @MainActor func testPortraitMaskFromToolbar() throws {
        let app=XCUIApplication();app.launchArguments=["--portrait-demo"];app.launch();waitForSave(app)
        app.buttons["section-Retusche"].tap();app.buttons["auto-Haar"].tap();waitForSave(app)
        XCTAssertTrue(app.buttons["Haar · Person 1"].waitForExistence(timeout:40),app.debugDescription)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="portrait-hair-mask";shot.lifetime = .keepAlways;add(shot)
        app.scrollViews["settings-panel"].swipeUp()
        let exposure=app.sliders["mask-Belichtung"]
        XCTAssertTrue(exposure.waitForExistence(timeout:10));exposure.adjust(toNormalizedSliderPosition:0.7);waitForSave(app)
        exposure.doubleTap();waitForSave(app)
        XCTAssertTrue((exposure.value as? String ?? "").contains("50"))
    }
    @MainActor func testAutomaticMasks() throws {
        let app=XCUIApplication();app.launchArguments=["--verify-ai"];app.launch()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","KI geprüft: 4 Bereiche")).firstMatch.waitForExistence(timeout:180),app.debugDescription)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        selectTool("Masken",in:app)
        XCTAssertTrue(app.buttons["KI-Prüfmaske"].exists)
        app.buttons["Maske löschen"].tap();waitForSave(app)
        XCTAssertFalse(app.buttons["KI-Prüfmaske"].exists)
        app.buttons["Rückgängig"].tap();waitForSave(app)
        XCTAssertTrue(app.buttons["KI-Prüfmaske"].exists)
    }
    @MainActor func testLookThumbnailsAndPhotoGestures() throws {
        let app=XCUIApplication();app.launchArguments=["--demo","--verify-gestures"];app.launch()
        waitForSave(app)
        let exposure=app.sliders["slider-exposure"]
        XCTAssertTrue(exposure.waitForExistence(timeout:30));exposure.adjust(toNormalizedSliderPosition:0.65);waitForSave(app)
        let point=app.coordinate(withNormalizedOffset:CGVector(dx:0.5,dy:0.35))
        point.press(forDuration:1)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Vergleich geprüft")).firstMatch.waitForExistence(timeout:10))
        XCTAssertFalse(app.staticTexts["before-indicator"].exists)
        let handle=app.buttons["panel-handle"]
        let begin=handle.coordinate(withNormalizedOffset:CGVector(dx:0.5,dy:0.5))
        begin.press(forDuration:0.05,thenDragTo:begin.withOffset(CGVector(dx:0,dy:150)))
        XCTAssertTrue(app.buttons["Regler einblenden"].waitForExistence(timeout:10))
        XCTAssertFalse(app.sliders["slider-exposure"].exists)
        let full=XCTAttachment(screenshot:XCUIScreen.main.screenshot());full.name="full-photo";full.lifetime = .keepAlways;add(full)
        point.press(forDuration:1)
        XCTAssertFalse(app.staticTexts["before-indicator"].exists)
        let bottom=app.buttons["panel-handle"].coordinate(withNormalizedOffset:CGVector(dx:0.5,dy:0.5))
        bottom.press(forDuration:0.05,thenDragTo:bottom.withOffset(CGVector(dx:0,dy:-150)))
        XCTAssertTrue(app.sliders["slider-exposure"].waitForExistence(timeout:10))
        selectTool("Looks",in:app)
        XCTAssertTrue(app.images["look-preview-Natürlich"].waitForExistence(timeout:30))
        app.scrollViews["settings-panel"].swipeUp()
        XCTAssertTrue(app.images["look-preview-Teal & Amber"].waitForExistence(timeout:30))
        let looks=XCTAttachment(screenshot:XCUIScreen.main.screenshot());looks.name="real-look-thumbnails";looks.lifetime = .keepAlways;add(looks)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testDeleteMasksAndRestore() throws {
        let app=XCUIApplication(); app.launchArguments=["--demo"]; app.launch()
        waitForSave(app); selectTool("Masken",in:app)
        for name in ["Radialer Verlauf","Linearer Verlauf","Pinsel"] {
            app.buttons["new-mask"].tap(); app.buttons[name].tap(); waitForSave(app)
        }
        app.buttons["Verlauf 2"].tap()
        app.buttons["Maske löschen"].tap(); waitForSave(app)
        XCTAssertFalse(app.buttons["Verlauf 2"].exists)
        XCTAssertTrue(app.buttons["Pinsel 3"].exists)
        app.buttons["Maske löschen"].tap(); waitForSave(app)
        app.buttons["Maske löschen"].tap(); waitForSave(app)
        XCTAssertFalse(app.buttons["Maske löschen"].exists)
        XCTAssertEqual(app.state,.runningForeground)
        app.buttons["Rückgängig"].tap(); waitForSave(app)
        XCTAssertTrue(app.buttons["Maske löschen"].waitForExistence(timeout:10))
        app.buttons["Maske löschen"].tap(); waitForSave(app)
        XCTAssertFalse(app.buttons["Maske löschen"].exists)
        app.terminate(); app.launchArguments=[]; app.launch(); waitForSave(app)
        selectTool("Masken",in:app)
        XCTAssertFalse(app.buttons["Maske löschen"].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testCinemaLooks() throws {
        let app=XCUIApplication(); app.launchArguments=["--demo","--verify-cinema"]; app.launch()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Cinema geprüft")).firstMatch.waitForExistence(timeout:90),app.debugDescription)
        selectTool("Masken",in:app)
        app.buttons["new-mask"].tap(); app.buttons["Radialer Verlauf"].tap(); waitForSave(app)
        selectTool("Looks",in:app)
        let preset=app.buttons["cinema-Teal & Amber"]
        app.scrollViews["settings-panel"].swipeUp()
        XCTAssertTrue(preset.waitForExistence(timeout:10)); preset.tap(); waitForSave(app)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot());shot.name="cinema-looks";shot.lifetime = .keepAlways;add(shot)
        selectTool("Masken",in:app)
        XCTAssertTrue(app.buttons["Radial 1"].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testDevelopmentAndMasks() throws {
        let app=XCUIApplication(); app.launchArguments=["--demo","--verify-development"]; app.launch()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@","Bildprüfung bestanden")).firstMatch.waitForExistence(timeout:180),app.debugDescription)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); shot.name="studio-v4-light"; shot.lifetime = .keepAlways; add(shot)
        selectTool("Farbe",in:app)
        app.buttons["Farbmix"].tap()
        XCTAssertTrue(app.sliders["mix-Farbton"].waitForExistence(timeout:10))
        selectTool("Masken",in:app)
        XCTAssertTrue(app.buttons["new-mask"].waitForExistence(timeout:10))
        app.buttons["new-mask"].tap()
        app.buttons["Pinsel"].tap()
        let start=app.coordinate(withNormalizedOffset:CGVector(dx:0.35,dy:0.30))
        let end=app.coordinate(withNormalizedOffset:CGVector(dx:0.65,dy:0.40))
        start.press(forDuration:0.1,thenDragTo:end)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        let maskshot=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); maskshot.name="studio-v4-masks"; maskshot.lifetime = .keepAlways; add(maskshot)
    }
    @MainActor func testRepeatedPhotoImport() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--demo"]
        app.launch()
        for _ in 0..<2 {
            let library = app.buttons["Mediathek"]
            XCTAssertTrue(library.waitForExistence(timeout:90))
            expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:library)
            waitForExpectations(timeout:60)
            library.tap()
            let add = app.buttons["library-import-photo"]
            XCTAssertTrue(add.waitForExistence(timeout:10))
            add.tap()
            let photo = app.images.matching(NSPredicate(format:"label CONTAINS[c] 'Photo' OR label CONTAINS[c] 'Foto'")).firstMatch
            XCTAssertTrue(photo.waitForExistence(timeout:30), app.debugDescription)
            photo.tap()
            let photos=app.images.matching(NSPredicate(format:"label CONTAINS[c] 'Photo' OR label CONTAINS[c] 'Foto'"))
            XCTAssertGreaterThanOrEqual(photos.count,2,app.debugDescription)
            photos.element(boundBy:1).tap()
            let confirm=app.buttons.matching(NSPredicate(format:"label == 'Add' OR label == 'Hinzufügen' OR label == 'Fertig' OR label == 'Done'")).firstMatch
            XCTAssertTrue(confirm.waitForExistence(timeout:10),app.debugDescription);confirm.tap()
            XCTAssertTrue(app.sliders["slider-exposure"].waitForExistence(timeout:60))
            expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:library)
            waitForExpectations(timeout:60)
            XCTAssertTrue(app.buttons["Nächstes Foto"].waitForExistence(timeout:30))
            app.buttons["Nächstes Foto"].tap();waitForSave(app)
            XCTAssertTrue(app.buttons["Vorheriges Foto"].isEnabled)
            XCTAssertFalse(app.alerts.firstMatch.exists)
        }
    }

    @MainActor func testStudioEditingAndExport() throws {
        let app=XCUIApplication()
        app.launchArguments=["--demo"]
        app.launch()
        let exposure=app.sliders["slider-exposure"]
        XCTAssertTrue(exposure.waitForExistence(timeout:90))
        let saved=NSPredicate(format:"enabled == true")
        expectation(for:saved,evaluatedWith:exposure)
        waitForExpectations(timeout:60)
        let initial=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); initial.name="01-studio-light"; initial.lifetime = .keepAlways; add(initial)
        exposure.adjust(toNormalizedSliderPosition:0.60)
        let undo=app.buttons["Rückgängig"]
        expectation(for:saved,evaluatedWith:undo)
        waitForExpectations(timeout:60)
        selectTool("Farbe",in:app)
        let warmth=app.sliders["slider-warmth"]
        XCTAssertTrue(warmth.waitForExistence(timeout:10))
        warmth.adjust(toNormalizedSliderPosition:0.68)
        expectation(for:saved,evaluatedWith:undo)
        waitForExpectations(timeout:60)
        let color=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); color.name="02-transparent-color"; color.lifetime = .keepAlways; add(color)
        selectTool("Zuschnitt",in:app)
        app.buttons["crop-ratios"].tap();app.buttons["1:1"].tap()
        let crop=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); crop.name="03-free-crop"; crop.lifetime = .keepAlways; add(crop)
        app.buttons["Anwenden"].tap()
        expectation(for:saved,evaluatedWith:undo)
        waitForExpectations(timeout:60)
        undo.tap()
        expectation(for:saved,evaluatedWith:app.buttons["Exportieren und speichern"])
        waitForExpectations(timeout:60)
        selectTool("Looks",in:app)
        app.buttons["Golden"].tap()
        expectation(for:saved,evaluatedWith:app.buttons["Exportieren und speichern"])
        waitForExpectations(timeout:60)
        app.buttons["Exportieren und speichern"].tap()
        app.buttons["export-format"].tap();app.buttons["PNG"].tap();app.swipeUp();app.buttons["export-share"].tap()
        XCTAssertTrue(app.otherElements["ActivityListView"].waitForExistence(timeout:30) || app.buttons["Copy"].exists || app.buttons["Kopieren"].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testFullResolutionImport() throws {
        let app=XCUIApplication(); app.launchArguments=["--demo48"]; app.launch()
        XCTAssertTrue(app.sliders["slider-exposure"].waitForExistence(timeout:180))
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:app.buttons["Exportieren und speichern"])
        waitForExpectations(timeout:180)
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format:"label CONTAINS %@", "6048 × 8064")).firstMatch.exists)
        app.buttons["Exportieren und speichern"].tap()
        app.swipeUp();app.buttons["export-share"].tap()
        XCTAssertTrue(app.otherElements["ActivityListView"].waitForExistence(timeout:180) || app.buttons["Copy"].exists || app.buttons["Kopieren"].exists)
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }
    @MainActor func testLibraryAndLandscape() throws {
        let app=XCUIApplication(); app.launchArguments=["--demo"]; app.launch()
        XCTAssertTrue(app.sliders["slider-exposure"].waitForExistence(timeout:90))
        expectation(for:NSPredicate(format:"enabled == true"),evaluatedWith:app.buttons["Mediathek"])
        waitForExpectations(timeout:60)
        app.buttons["Mediathek"].tap()
        XCTAssertTrue(app.navigationBars["Dein Studio"].waitForExistence(timeout:10))
        app.buttons["Fertig"].tap()
        XCUIDevice.shared.orientation = .landscapeLeft
        expectation(for:NSPredicate { _,_ in app.windows.firstMatch.frame.width > app.windows.firstMatch.frame.height },evaluatedWith:app)
        waitForExpectations(timeout:15)
        sleep(2)
        let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot()); shot.name="04-landscape"; shot.lifetime = .keepAlways; add(shot)
        XCTAssertTrue(app.buttons["Exportieren und speichern"].isHittable)
        XCUIDevice.shared.orientation = .portrait
    }
}
