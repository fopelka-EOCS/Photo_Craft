import Foundation
import Combine
import UIKit
import ImageIO
import CoreImage
import UniformTypeIdentifiers

struct LayerInfo: Decodable, Identifiable {
    let id: UInt64
    let name: String
    let visible: Bool
    let opacity: Double
    let kind: String
}
struct DocumentInfo: Decodable {
    let width: Int
    let height: Int
    let layers: [LayerInfo]
    let activeLayer: UInt64?
    let canUndo: Bool
    let canRedo: Bool
}
struct EngineReply: Decodable {
    let ok: Bool
    let error: String?
    let document: DocumentInfo?
    let preview: String?
    let path: String?
    let nativeState: NativeState?
    let sourceType: String?
}
struct EditorError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

// Every engine request, image decode and disk write runs off the UI thread.
private let engineQueue = DispatchQueue(label: "photocraft.engine", qos: .userInitiated)
private func request(_ values: [String: Any]) throws -> EngineReply {
    let data = try JSONSerialization.data(withJSONObject: values)
    guard let text = String(data: data, encoding: .utf8) else {
        throw EditorError(message: "Ungültige Anfrage.")
    }
    let result: Data = try text.withCString { input in
        guard let output = pc_request(input) else { throw EditorError(message: "Die Bildbearbeitung konnte nicht antworten.") }
        defer { pc_free(output) }
        return Data(String(cString: output).utf8)
    }
    let reply = try JSONDecoder().decode(EngineReply.self, from: result)
    guard reply.ok else { throw EditorError(message: reply.error ?? "Bearbeitung fehlgeschlagen.") }
    return reply
}
private func background<T>(_ work: @escaping () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
        engineQueue.async {
            do { continuation.resume(returning: try work()) }
            catch { continuation.resume(throwing: error) }
        }
    }
}

struct Development: Codable, Hashable {
    var exposure = 0.0
    var brightness = 0.0
    var contrast = 0.0
    var saturation = 0.0
    var vibrance = 0.0
    var warmth = 0.0
    var tint = 0.0
    var lens = 0.0
    var redFringe = 0.0
    var blueFringe = 0.0
    var mix = Array(repeating:ColorBand(),count:8)
    var highlights = 0.0
    var shadows = 0.0
    var whites = 0.0
    var blacks = 0.0
    var texture = 0.0
    var clarity = 0.0
    var dehaze = 0.0
    var vignette = 0.0
    var vignetteMid = 50.0
    var grain = 0.0
    var sharpness = 0.0
    var sharpRadius = 1.0
    var sharpDetail = 25.0
    var sharpMask = 0.0
    var noise = 0.0
    var noiseDetail = 50.0
    var noiseContrast = 0.0
    var colorNoise = 0.0
    var colorDetail = 50.0
    var colorSmooth = 50.0
    var blur = 0.0
    var hue = 0.0
    var fade = 0.0
    var gradeWarm = 0.0
    var gradeCool = 0.0
    var curveShadows = 0.0
    var curveMid = 0.0
    var curveHighlights = 0.0
    var values: [String: Any] {
        (try? JSONSerialization.jsonObject(with: JSONEncoder().encode(self))) as? [String:Any] ?? [:]
    }
}
enum CinemaLook: String, CaseIterable {
    case teal = "Teal & Amber", bleach = "Bleach Bypass", golden = "Golden Film", midnight = "Midnight"
    case desert = "Desert", soft = "Soft Cinema", noir = "Neo Noir", vintage = "Vintage 35"
    var subtitle:String {
        switch self {
        case .teal: "Kühle Tiefen, warme Lichter"
        case .bleach: "Entsättigt und kontrastreich"
        case .golden: "Warmes Licht, sanfte Höhen"
        case .midnight: "Kühl und zurückgenommen"
        case .desert: "Sandtöne und matte Schatten"
        case .soft: "Weiche Kontraste, zarte Farben"
        case .noir: "Monochrom mit Filmkorn"
        case .vintage: "Analoge Farben und Patina"
        }
    }
    var settings:Development {
        var d=Development()
        switch self {
        case .teal:
            d.contrast=18; d.highlights = -28; d.shadows=12; d.saturation = -10; d.vibrance=16
            d.gradeWarm=25; d.gradeCool=32; d.fade=10; d.vignette = -14
            d.mix[4].hue = -25; d.mix[5].hue = -35; d.mix[3].saturation = -24
        case .bleach:
            d.contrast=34; d.highlights = -30; d.shadows = -12; d.saturation = -48; d.clarity=18; d.grain=9; d.vignette = -16
        case .golden:
            d.exposure=0.15; d.warmth=18; d.highlights = -35; d.shadows=16; d.gradeWarm=32; d.fade=12; d.saturation = -8; d.grain=6
        case .midnight:
            d.exposure = -0.25; d.warmth = -17; d.contrast=22; d.blacks = -12; d.highlights = -22; d.gradeCool=40; d.saturation = -24; d.vignette = -25
        case .desert:
            d.warmth=22; d.contrast=12; d.highlights = -24; d.fade=22; d.saturation = -18; d.grain=8
            d.mix[3].hue = -45; d.mix[3].saturation = -45; d.mix[5].saturation = -30
        case .soft:
            d.exposure=0.1; d.contrast = -16; d.highlights = -26; d.shadows=18; d.fade=25; d.clarity = -8; d.vibrance=8; d.saturation = -10
        case .noir:
            d.saturation = -100; d.contrast=32; d.highlights = -25; d.shadows = -10; d.blacks = -12; d.grain=16; d.vignette = -24; d.curveMid=8
        case .vintage:
            d.warmth=12; d.tint=5; d.fade=32; d.contrast=8; d.highlights = -30; d.saturation = -20; d.gradeWarm=18; d.grain=14; d.vignette = -18
            d.mix[3].hue = -20; d.mix[5].saturation = -18
        }
        return d
    }
}
struct SavedProject: Identifiable {
    let url: URL
    let date: Date
    var id: URL { url }
    var thumbnail: UIImage? { UIImage(contentsOfFile: url.deletingPathExtension().appendingPathExtension("jpg").path) }
}
@MainActor final class EditorModel: ObservableObject {
    @Published var catalog=PhotoCatalog()
    @Published var batchRunning=false
    var spatialEditing=false
    @Published var document: DocumentInfo?
    @Published var preview: UIImage?
    @Published var development = Development()
    @Published var originalPreview: UIImage?
    @Published var projects: [SavedProject] = []
    @Published var previewRendering = false
    private var previewTask: Task<Void,Never>?
    private var pendingPreview: NativeState?
    @Published var rawSettings: RAWSettings?
    @Published var advanced = AdvancedSettings()
    private var sourceEdits:[SourceEdit]?
    private var originalURL:URL?
    private var importedAspectRatio:Double?
    var originalAspectRatio:Double {
        let ratio=importedAspectRatio ?? Double(document?.width ?? 1)/Double(document?.height ?? 1)
        let turns=sourceEdits?.filter{$0.kind == "image.imageRotation.90cw"}.count ?? 0
        return turns % 2 == 0 ? ratio:1/ratio
    }
    @Published private(set) var sourceType:String?
    @Published var masks: [LocalMask] = []
    @Published var selectedMask: String?
    private var basePreview: CGImage?
    @Published private(set) var lookThumbnails: [String:UIImage] = [:]
    @Published private(set) var lookPreviewRevision = 0
    private var pendingLookThumbnails: [String:Int] = [:]
    private var committedState = NativeState()
    private var deferredCommit: Task<Void,Never>?
    private let renderQueue = DispatchQueue(label:"photocraft.gpu",qos:.userInteractive)
    private var previewGeneration = 0
    private var committedDevelopment = Development()
    private var committedPreview: UIImage?
    @Published var segmenting = false
    private let segmentationQueue=DispatchQueue(label:"photocraft.segmentation",qos:.userInitiated)
    @Published var busy = false
    @Published var error: String?
    @Published var status = "Lokal auf deinem iPhone"
    @Published var shareURL: URL?
    @Published private(set) var projectURL: URL?
    @Published var editingProjects:[URL]=[]
    var selectionIndex:Int { editingProjects.firstIndex(of:projectURL ?? URL(fileURLWithPath:"/")) ?? 0 }
    func openSelection(_ urls:[URL]) async {
        guard !busy,let first=urls.first else {return}
        await openProject(first)
        if projectURL == first {editingProjects=urls}
    }
    func moveSelection(_ offset:Int) async {
        let next=selectionIndex+offset
        guard !busy,editingProjects.indices.contains(next) else {return}
        await openProject(editingProjects[next])
    }
    private var didRestore = false
    private var documentsURL: URL { FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0] }

    private func accept(_ reply: EngineReply) {
        if let document = reply.document { self.document = document }
        let state = reply.nativeState ?? NativeState()
        development = state.development; masks = state.masks
        rawSettings=state.raw;sourceEdits=state.sourceEdits;advanced=state.advanced ?? AdvancedSettings();sourceType=reply.sourceType
        if !masks.contains(where:{$0.id == selectedMask}) { selectedMask=masks.last?.id }
        committedState = state; committedDevelopment = development
        if let encoded = reply.preview, let data = Data(base64Encoded:encoded), let image=UIImage(data:data), let cg=image.cgImage {
            basePreview=cg; originalPreview=image
            invalidateLookThumbnails()
            preview=image; committedPreview=image
            previewChanges()
        }
    }
    var nativeState: NativeState { NativeState(development:development,masks:masks,raw:rawSettings,sourceEdits:sourceEdits,advanced:advanced) }
    private func render(_ base:CGImage,_ state:NativeState) async throws -> UIImage {
        let original=originalURL
        var displayed=state
        if spatialEditing { displayed.advanced?.angle=0;displayed.advanced?.vertical=0;displayed.advanced?.horizontal=0 }
        let renderState=displayed
        return try await withCheckedThrowingContinuation { continuation in
            renderQueue.async {
                do { continuation.resume(returning:try DevelopRenderer.shared.renderOriginal(original,fallback:base,state:renderState)) }
                catch { continuation.resume(throwing:error) }
            }
        }
    }
    private func report(_ failure: Error) { error = failure.localizedDescription; status = "Nicht gespeichert – bitte erneut versuchen" }

    func restore() async {
        guard !didRestore else { return }; didRestore = true
        loadCatalog();refreshProjects()
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--verify-crop-ai") {await loadDemo();do {try await ExpansionChecks.run();status="Zuschnitt und KI-Anbindung geprüft"}catch{report(error)};return}
        if ProcessInfo.processInfo.arguments.contains("--verify-export") { await loadDemo();await verifyExport();return }
        if ProcessInfo.processInfo.arguments.contains("--verify-presets") { await loadDemo();await verifyPresets();return }
        if ProcessInfo.processInfo.arguments.contains("--verify-raw") { await verifyRAW();return }
        if ProcessInfo.processInfo.arguments.contains("--verify-advanced") { await loadDemo();await verifyAdvanced();return }
        #endif
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--portrait-demo"),let data=try? Data(contentsOf:documentsURL.appendingPathComponent("astronaut.png")) { await importPhoto(data);return }
        if ProcessInfo.processInfo.arguments.contains("--verify-ai-device") { await verifyAutomaticMasks();return }
        if ProcessInfo.processInfo.arguments.contains("--verify-ai") {
            await loadDemo();await verifyAutomaticMasks();return
        }
        #endif
        if ProcessInfo.processInfo.arguments.contains("--demo") || ProcessInfo.processInfo.arguments.contains("--demo48") {
            await loadDemo(); if ProcessInfo.processInfo.arguments.contains("--verify-cinema") { await verifyCinema() }; if ProcessInfo.processInfo.arguments.contains("--verify-development") { await verifyDevelopment() }; return
        }
        guard let name = UserDefaults.standard.string(forKey: "lastProject") else { return }
        let url = documentsURL.appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        await openProject(url)
        if ProcessInfo.processInfo.arguments.contains("--benchmark-only") { await benchmarkPreview() }
    }
    func openProject(_ url: URL) async {
        guard !busy else { return }; invalidatePreview(); busy = true; defer { busy = false }
        do {
            try await saveCurrent()
            let reply = try await background { try request(["op":"open", "path":url.path]) }
            try await loadOriginal(reply)
            accept(reply); projectURL = url
            UserDefaults.standard.set(url.lastPathComponent, forKey: "lastProject")
            status = "Projekt geöffnet"
        } catch { report(error) }
    }
    private func loadOriginal(_ reply:EngineReply) async throws {
        let previous=originalURL
        if let type=reply.sourceType {
            let ext=UTType(type)?.preferredFilenameExtension ?? "raw"
            let url=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString+"."+ext)
            _ = try await background { try request(["op":"extractSource","path":url.path]) }
            originalURL=url
            if let source=CGImageSourceCreateWithURL(url as CFURL,nil),let info=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],let w=info[kCGImagePropertyPixelWidth] as? Double,let h=info[kCGImagePropertyPixelHeight] as? Double,w>0,h>0 {
                let orientation=info[kCGImagePropertyOrientation] as? Int ?? 1
                importedAspectRatio=(5...8).contains(orientation) ? h/w:w/h
            } else {importedAspectRatio=nil}
        } else { originalURL=nil;importedAspectRatio=nil }
        if let previous { try? FileManager.default.removeItem(at:previous) }
    }
    func importPhoto(_ data: Data) async {
        guard !busy else { return }; invalidatePreview(); busy = true; defer { busy = false }
        let folder = documentsURL
        let newProject = folder.appendingPathComponent("Foto-\(UUID().uuidString).pcraft")
        do {
            try await saveCurrent()
            let reply = try await background {
                guard data.count <= 128 * 1024 * 1024,
                      let source = CGImageSourceCreateWithData(data as CFData, nil) else {
                    throw EditorError(message: "Dieses Foto kann nicht geöffnet werden (Original maximal 128 MB).")
                }
                let type=(CGImageSourceGetType(source) as String?) ?? UTType.data.identifier
                let original=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString+"."+(UTType(type)?.preferredFilenameExtension ?? "raw"))
                try data.write(to:original,options:.atomic)
                defer { try? FileManager.default.removeItem(at:original) }
                let isRAW=UTType(type)?.conforms(to:.rawImage) == true || type.lowercased().contains("raw") || type.lowercased().contains("cr3") || type.lowercased().contains("dng")
                let raw=isRAW ? OriginalPipeline.rawSettings(original) : nil
                if isRAW && raw == nil { throw EditorError(message:"Dieses Kamera-RAW wird von dieser iOS-Version nicht unterstützt.") }
                let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
                let w = (properties?[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue ?? 8192
                let h = (properties?[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue ?? 8192
                let largest = max(w,h)
                let scale = min(1.0, min(8192 / max(1,largest), sqrt(50_000_000 / max(1,w*h))))
                let maximumSide = max(1,Int(largest * scale))
                let options: [CFString: Any] = [
                    kCGImageSourceCreateThumbnailFromImageAlways: true,
                    kCGImageSourceCreateThumbnailWithTransform: true,
                    kCGImageSourceThumbnailMaxPixelSize: maximumSide,
                    kCGImageSourceShouldCacheImmediately: true
                ]
                let decoded:CGImage?
                if let raw {
                    let image=try OriginalPipeline.image(original,state:NativeState(raw:raw),maxSide:CGFloat(maximumSide))
                    decoded=DevelopRenderer.shared.context.createCGImage(image,from:image.extent,format:.RGBA8,colorSpace:CGColorSpace(name:CGColorSpace.sRGB)!)
                } else { decoded=CGImageSourceCreateThumbnailAtIndex(source,0,options as CFDictionary) }
                guard let image=decoded else {
                    throw EditorError(message: "Das Bildformat wird nicht unterstützt.")
                }
                let input = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
                defer { try? FileManager.default.removeItem(at: input) }
                guard let destination = CGImageDestinationCreateWithURL(input as CFURL, UTType.png.identifier as CFString, 1, nil) else {
                    throw EditorError(message: "Arbeitskopie konnte nicht erstellt werden.")
                }
                CGImageDestinationAddImage(destination, image, nil)
                guard CGImageDestinationFinalize(destination) else { throw EditorError(message: "Arbeitskopie konnte nicht gespeichert werden.") }
                _ = try request(["op":"open", "path":input.path])
                let state=NativeState(raw:raw,sourceEdits:[],advanced:AdvancedSettings())
                let object=try JSONSerialization.jsonObject(with:JSONEncoder().encode(state))
                return try request(["op":"attachSource","path":original.path,"type":type,"state":object])
            }
            try await loadOriginal(reply)
            accept(reply); projectURL = newProject
            // Keep the new document visible and retryable even if its first save fails.
            try await saveCurrent()
            UserDefaults.standard.set(newProject.lastPathComponent, forKey: "lastProject")
        } catch { report(error) }
    }
    func importFile(_ url: URL) async {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard size <= 384 * 1024 * 1024 else { throw EditorError(message: "Maximal 384 MB pro Datei.") }
            if url.pathExtension.lowercased() == "pcraft" {
                let local = documentsURL.appendingPathComponent("Projekt-\(UUID().uuidString).pcraft")
                try FileManager.default.copyItem(at: url, to: local)
                await openProject(local)
            } else {
                let data = try await background { try Data(contentsOf: url) }
                await importPhoto(data)
            }
        } catch { report(error) }
    }
    private func saveCurrent() async throws {
        guard let url = projectURL else { return }
        _ = try await background { try request(["op":"save", "path":url.path]) }
        UserDefaults.standard.set(url.lastPathComponent, forKey: "lastProject")
        if let jpeg = committedPreview?.jpegData(compressionQuality: 0.65) {
            let thumbnail = url.deletingPathExtension().appendingPathExtension("jpg")
            try? await background { try jpeg.write(to: thumbnail, options: .atomic) }
        }
        refreshProjects()
        status = "Gespeichert"
    }
    func save() async {
        guard !busy, document != nil else { return }; busy = true; defer { busy = false }
        do { try await saveCurrent() } catch { report(error) }
    }
    func command(_ id: String, _ params: [String: Any] = [:]) async {
        guard !busy else { return }
        if originalURL != nil && id.hasPrefix("layer.") { error="Original-Entwicklung verwendet eine Bildebene. Für Ebenenbearbeitung zuerst als TIFF/PNG exportieren und in einem separaten Projekt öffnen.";return }
        invalidatePreview(); busy = true; defer { busy = false }
        do {
            let reply = try await background { try request(["op":"command", "id":id, "params":params]) }
            accept(reply)
            try await saveCurrent()
        } catch { report(error) }
    }
    func refreshProjects() {
        let files = (try? FileManager.default.contentsOfDirectory(at: documentsURL, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        projects = files.filter { $0.pathExtension == "pcraft" }.map {
            SavedProject(url: $0, date: (try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast)
        }.sorted { $0.date > $1.date }
    }
    private func invalidatePreview() {
        previewGeneration += 1
        pendingPreview = nil
    }
    func previewChanges() {
        pendingPreview = nativeState
        guard previewTask == nil, basePreview != nil else { return }
        previewTask = Task { [weak self] in
            guard let self else { return }
            self.previewRendering = true
            defer { self.previewRendering = false; self.previewTask = nil }
            while let state = self.pendingPreview, let base=self.basePreview {
                self.pendingPreview = nil
                let generation=self.previewGeneration
                do {
                    let image=try await self.render(base,state)
                    // Display completed frames even while a newer frame is pending.
                    // Previously continuous dragging discarded every completed frame.
                    if generation == self.previewGeneration {
                        self.preview=image
                        if state == self.committedState { self.committedPreview=image }
                    }
                } catch { if generation == self.previewGeneration { self.error=error.localizedDescription } }
            }
        }
    }
    func commitDevelopment() async {
        if busy {
            if deferredCommit == nil {
                deferredCommit=Task { [weak self] in
                    guard let self else { return }
                    while self.busy { try? await Task.sleep(nanoseconds:30_000_000) }
                    self.deferredCommit=nil
                    await self.commitDevelopment()
                }
            }
            return
        }
        let state=nativeState
        guard state != committedState else { return }
        busy=true; defer { busy=false }
        do {
            let encoded=try JSONEncoder().encode(state)
            let object=try JSONSerialization.jsonObject(with:encoded)
            let reply=try await background { try request(["op":"native","state":object]) }
            if let doc=reply.document { document=doc }
            if committedState.masks != state.masks || committedState.advanced != state.advanced || committedState.raw != state.raw { invalidateLookThumbnails() }
            committedState=state; committedDevelopment=development
            if let base=basePreview { let image=try await render(base,state); if nativeState == state { preview=image }; committedPreview=image }
            try await saveCurrent()
        } catch {
            development=committedState.development; masks=committedState.masks;rawSettings=committedState.raw;advanced=committedState.advanced ?? AdvancedSettings()
            preview=committedPreview; report(error)
        }
    }
    func referencePreview(_ values:Development) async throws -> UIImage {
        guard let base=basePreview else {throw EditorError(message:"Bitte zuerst ein Foto öffnen.")}
        var state=nativeState;state.development=values
        return try await render(base,state)
    }
    func applyPreset(_ values: Development) async {
        development=values; previewChanges(); await commitDevelopment()
    }
    private func invalidateLookThumbnails() {
        lookPreviewRevision += 1; lookThumbnails=[:]; pendingLookThumbnails=[:]
    }
    func loadLookThumbnail(_ name:String,settings:Development) async {
        guard lookThumbnails[name] == nil, pendingLookThumbnails[name] == nil, let base=basePreview else { return }
        let revision=lookPreviewRevision
        var state=nativeState;state.development=settings
        let original=originalURL
        pendingLookThumbnails[name]=revision
        let result:UIImage? = await withCheckedContinuation { continuation in
            renderQueue.async {
                let source=CIImage(cgImage:base)
                let scale=min(1,256/CGFloat(max(base.width,base.height)))
                let small=source.transformed(by:CGAffineTransform(scaleX:scale,y:scale))
                let renderer=DevelopRenderer.shared
                guard let cg=renderer.context.createCGImage(small,from:small.extent) else { continuation.resume(returning:nil);return }
                continuation.resume(returning:try? renderer.renderOriginal(original,fallback:cg,state:state,maxSide:256))
            }
        }
        guard revision == lookPreviewRevision else { return }
        pendingLookThumbnails[name]=nil
        if let result { lookThumbnails[name]=result }
    }
    func compare() async { if let base=basePreview { originalPreview=UIImage(cgImage:base) } }
    func sampleWhiteBalance(_ point:MaskPoint) async {
        guard let base=basePreview else { return }
        do {
            let rgb:[Double] = try await withCheckedThrowingContinuation { continuation in
                renderQueue.async {
                    let input=CIImage(cgImage:base)
                    let x=point.x*Double(base.width); let y=(1-point.y)*Double(base.height)
                    let patch=CGRect(x:max(0,min(Double(base.width)-3,x-1)),y:max(0,min(Double(base.height)-3,y-1)),width:3,height:3)
                    let average=input.applyingFilter("CIAreaAverage",parameters:[kCIInputExtentKey:CIVector(cgRect:patch)])
                    var bytes=[UInt8](repeating:0,count:4)
                    DevelopRenderer.shared.context.render(average,toBitmap:&bytes,rowBytes:4,bounds:CGRect(x:0,y:0,width:1,height:1),format:.RGBA8,colorSpace:CGColorSpace(name:CGColorSpace.sRGB))
                    continuation.resume(returning:bytes.prefix(3).map { Double($0)/255 })
                }
            }
            guard rgb.min() ?? 0 > 0.02 else { throw EditorError(message:"Diese Stelle ist zu dunkel für den Weißabgleich.") }
            development.warmth=min(100,max(-100,(rgb[2]-rgb[0])/max(0.01,rgb[0]+rgb[2])/0.17*100))
            development.tint=min(100,max(-100,(1-(rgb[0]+rgb[2])/2/rgb[1])/0.12*100))
            previewChanges(); await commitDevelopment()
        } catch { report(error) }
    }
    func generateMask(_ region:AutomaticRegion) async {
        guard !busy,!segmenting,let base=basePreview else { return }
        guard masks.count<12 else { error="Maximal zwölf Masken pro Foto.";return }
        await commitDevelopment()
        guard !busy else { return }
        busy=true;segmenting=true;status="\(region.rawValue) wird erkannt …"
        do {
            let created:[LocalMask]=try await withCheckedThrowingContinuation { continuation in
                segmentationQueue.async {
                    do { continuation.resume(returning:try AutomaticMaskEngine.shared.generate(region,image:base)) }
                    catch { continuation.resume(throwing:error) }
                }
            }
            guard masks.count+created.count<=12 else { throw EditorError(message:"Für diese Auswahl werden \(created.count) freie Masken benötigt. Lösche zunächst nicht mehr benötigte Masken.") }
            masks.append(contentsOf:created);selectedMask=created.first?.id
            busy=false;segmenting=false
            previewChanges();await commitDevelopment()
        } catch { busy=false;segmenting=false;status="Auswahl nicht erstellt";report(error) }
    }
    #if DEBUG
    func verifyAutomaticMasks() async {
        busy=true;segmenting=true
        do {
            let folder=documentsURL
            let evidence:[String:Int]=try await withCheckedThrowingContinuation { continuation in
                segmentationQueue.async {
                    do {
                        var counts:[String:Int]=[:]
                        for region in AutomaticRegion.allCases {
                            #if targetEnvironment(simulator)
                            if [.subject,.background,.objects].contains(region) { continue }
                            #endif
                            let file=region == .sky ? "rocket.jpg" : "astronaut.png"
                            guard let image=UIImage(contentsOfFile:folder.appendingPathComponent(file).path)?.cgImage else { throw EditorError(message:"Testfoto fehlt") }
                            try region.rawValue.write(to:folder.appendingPathComponent("ai-step.txt"),atomically:true,encoding:.utf8)
                            let masks=try AutomaticMaskEngine.shared.generate(region,image:image)
                            guard masks.allSatisfy({$0.rasterPNG != nil && $0.rasterCorners?.count == 3}) else { throw EditorError(message:"Ungültige KI-Maske") }
                            counts[region.rawValue]=masks.count
                            for (i,mask) in masks.enumerated() {
                                let extent=CGRect(x:0,y:0,width:image.width,height:image.height)
                                let renderer=DevelopRenderer.shared
                                let matte=renderer.maskImage(mask,extent:extent)
                                guard let cg=renderer.context.createCGImage(matte,from:extent),let png=UIImage(cgImage:cg).pngData() else { throw EditorError(message:"Maskenrendering fehlgeschlagen") }
                                try png.write(to:folder.appendingPathComponent("mask-\(region.rawValue)-\(i).png"))
                                let decoded=try JSONDecoder().decode(LocalMask.self,from:JSONEncoder().encode(mask))
                                guard decoded == mask else { throw EditorError(message:"Masken-Roundtrip fehlgeschlagen") }
                            }
                        }
                        try JSONSerialization.data(withJSONObject:counts,options:.sortedKeys).write(to:folder.appendingPathComponent("ai-verification.json"))
                        continuation.resume(returning:counts)
                    } catch { continuation.resume(throwing:error) }
                }
            }
            busy=false;segmenting=false
            if ProcessInfo.processInfo.arguments.contains("--verify-ai-device") { status="KI geprüft: \(evidence.count) Bereiche";return }
            // Exercise persistence, undo and rendering with a real segmentation bitmap.
            let data=try Data(contentsOf:folder.appendingPathComponent("mask-Himmel-0.png"))
            var mask=LocalMask();mask.kind="ai";mask.name="KI-Prüfmaske";mask.rasterPNG=data;mask.rasterCorners=AutomaticMaskEngine.fullCorners;mask.exposure=0.5
            masks=[mask];selectedMask=mask.id;await commitDevelopment()
            await command("image.imageRotation.90cw")
            guard masks.first?.rasterCorners?[0] == MaskPoint(x:1,y:0) else { throw EditorError(message:"KI-Maske nicht mitgedreht") }
            await command("edit.undo")
            guard masks.first?.rasterPNG == data,masks.first?.rasterCorners == AutomaticMaskEngine.fullCorners else { throw EditorError(message:"KI-Maske nach Undo verändert") }
            if let url=projectURL { await openProject(url) }
            guard masks.first?.rasterPNG == data else { throw EditorError(message:"KI-Maske nicht gespeichert") }
            status="KI geprüft: \(evidence.count) Bereiche"
        } catch { busy=false;segmenting=false;try? String(describing:error).write(to:documentsURL.appendingPathComponent("ai-error.txt"),atomically:true,encoding:.utf8);report(error) }
    }
    #endif
    func addMask(_ kind:String) {
        guard masks.count < 12 else { error="Maximal zwölf Masken pro Foto."; return }
        var mask=LocalMask(); mask.kind=kind
        mask.name="\(kind == "brush" ? "Pinsel" : kind == "linear" ? "Verlauf" : "Radial") \(masks.count+1)"
        masks.append(mask); selectedMask=mask.id
        Task { await commitDevelopment() }
    }
    func removeMask() {
        masks.removeAll { $0.id == selectedMask }; selectedMask=masks.last?.id
        previewChanges(); Task { await commitDevelopment() }
    }
    func loadDemo() async {
        // Original procedural sample, used for simulator UI verification; no private photo.
        let fullResolution=ProcessInfo.processInfo.arguments.contains("--demo48")
        let size = fullResolution ? CGSize(width:6048,height:8064) : CGSize(width:1200,height:1600)
        let format=UIGraphicsImageRendererFormat(); format.scale=1
        let renderer = UIGraphicsImageRenderer(size:size,format:format)
        let image = renderer.image { ctx in
            ctx.cgContext.scaleBy(x:size.width/1200,y:size.height/1600)
            let colors = [UIColor(red:0.18,green:0.37,blue:0.42,alpha:1).cgColor,
                          UIColor(red:0.87,green:0.67,blue:0.43,alpha:1).cgColor] as CFArray
            if let gradient = CGGradient(colorsSpace:CGColorSpaceCreateDeviceRGB(),colors:colors,locations:[0,1]) {
                ctx.cgContext.drawLinearGradient(gradient,start:.zero,end:CGPoint(x:0,y:1600),options:[])
            }
            UIColor(red:0.99,green:0.82,blue:0.53,alpha:1).setFill()
            UIBezierPath(ovalIn:CGRect(x:760,y:270,width:180,height:180)).fill()
            for (index,color) in [UIColor(red:0.35,green:0.43,blue:0.40,alpha:1),UIColor(red:0.16,green:0.30,blue:0.30,alpha:1),UIColor(red:0.08,green:0.20,blue:0.22,alpha:1)].enumerated() {
                let y = CGFloat(640 + index*220)
                let path=UIBezierPath(); path.move(to:CGPoint(x:0,y:y+200))
                path.addCurve(to:CGPoint(x:1200,y:y+100),controlPoint1:CGPoint(x:350,y:y-400),controlPoint2:CGPoint(x:700,y:y+400))
                path.addLine(to:CGPoint(x:1200,y:1600)); path.addLine(to:CGPoint(x:0,y:1600)); path.close()
                color.setFill(); path.fill()
            }
        }
        if let data=image.pngData() { await importPhoto(data) }
    }
    func verifyCinema() async {
        do {
            guard let base=basePreview else { throw EditorError(message:"Testbild fehlt") }
            let neutral=try await render(base,NativeState())
            var outputs=Set<Data>()
            for look in CinemaLook.allCases {
                let output=try await render(base,NativeState(development:look.settings))
                guard let data=output.pngData(), data != neutral.pngData() else { throw EditorError(message:"Look ohne Wirkung: \(look.rawValue)") }
                outputs.insert(data)
            }
            guard outputs.count == CinemaLook.allCases.count else { throw EditorError(message:"Doppelte Cinema-Looks") }
            status="Cinema geprüft"
        } catch { report(error) }
    }
    func benchmarkPreview() async {
        guard let base=basePreview else { return }
        do {
            var samples:[Double]=[]
            for i in 0..<21 {
                var state=nativeState; state.development.exposure += Double(i)/100
                let start=CFAbsoluteTimeGetCurrent(); _ = try await render(base,state)
                if i>0 { samples.append((CFAbsoluteTimeGetCurrent()-start)*1000) }
            }
            let result:[String:Any] = ["meanMilliseconds":samples.reduce(0,+)/Double(samples.count),"width":base.width,"height":base.height,"samples":samples]
            try JSONSerialization.data(withJSONObject:result,options:.prettyPrinted).write(to:documentsURL.appendingPathComponent("preview-benchmark.json"))
        } catch { report(error) }
    }
    func verifyDevelopment() async {
        do {
            guard let base=basePreview else { throw EditorError(message:"Keine Testbildbasis") }
            let neutral=try await render(base,NativeState())
            var legacySamples:[Double]=[]
            for _ in 0..<3 {
                let values=Development().values
                let start=CFAbsoluteTimeGetCurrent()
                _ = try await background { try request(["op":"preview","values":values]) }
                legacySamples.append((CFAbsoluteTimeGetCurrent()-start)*1000)
            }
            var samples:[Double]=[]
            for i in 0..<12 {
                var state=NativeState(); state.development.exposure=Double(i)/12
                let start=CFAbsoluteTimeGetCurrent()
                let output=try await render(base,state)
                samples.append((CFAbsoluteTimeGetCurrent()-start)*1000)
                if i == 11 && output.pngData() == neutral.pngData() { throw EditorError(message:"Belichtungsfilter ohne Wirkung") }
            }
            let paths:[WritableKeyPath<Development,Double>] = [\.highlights,\.shadows,\.whites,\.blacks,\.texture,\.clarity,\.dehaze,\.sharpness,\.noise,\.colorNoise,\.vignette,\.grain,\.lens,\.redFringe,\.blueFringe,\.curveMid,\.blur,\.gradeWarm,\.gradeCool]
            for path in paths {
                var state=NativeState(); state.development[keyPath:path]=50
                let output=try await render(base,state)
                if output.pngData() == neutral.pngData() { throw EditorError(message:"Bildfilter ohne Wirkung") }
            }
            var mixed=NativeState(); mixed.development.mix[3].hue=80
            let mixedImage=try await render(base,mixed)
            guard mixedImage.pngData() != neutral.pngData() else { throw EditorError(message:"Farbmix ohne Wirkung") }
            for kind in ["radial","linear","brush"] {
                var mask=LocalMask(); mask.kind=kind; mask.exposure=1
                if kind == "brush" { mask.strokes=[MaskStroke(points:[MaskPoint(x:0.3,y:0.3),MaskPoint(x:0.7,y:0.7)],radius:0.08,erase:false)] }
                let output=try await render(base,NativeState(masks:[mask]))
                guard output.pngData() != neutral.pngData() else { throw EditorError(message:"Maske ohne Wirkung") }
            }
            var m=LocalMask(); m.exposure=1.5
            masks=[m]; selectedMask=m.id; development.highlights = -35
            await commitDevelopment()
            guard let url=projectURL else { throw EditorError(message:"Testprojekt fehlt") }
            let expected=nativeState
            await openProject(url)
            guard nativeState == expected else { throw EditorError(message:"Masken wurden nicht wiederhergestellt") }
            busy=true; development.shadows=5
            await commitDevelopment(); busy=false
            for _ in 0..<100 {
                if committedState.development.shadows == 5 && !busy { break }
                try await Task.sleep(nanoseconds:30_000_000)
            }
            guard committedState.development.shadows == 5 else { throw EditorError(message:"Wartende Änderung wurde nicht gespeichert") }
            let result:[String:Any] = ["passed":true,"previewWidth":base.width,"previewHeight":base.height,"meanMilliseconds":samples.dropFirst().reduce(0,+)/Double(samples.count-1),"legacyMeanMilliseconds":legacySamples.reduce(0,+)/Double(legacySamples.count),"samples":samples,"masksRestored":masks.count]
            let data=try JSONSerialization.data(withJSONObject:result,options:.prettyPrinted)
            try data.write(to:documentsURL.appendingPathComponent("development-verification.json"))
            status="Bildprüfung bestanden"
        } catch { report(error) }
    }
    @discardableResult func export(_ format: String, options:ExportOptions=ExportOptions(), present:Bool=true) async -> URL? {
        guard !busy else { return nil }; invalidatePreview(); busy = true; defer { busy = false }
        let ext=format == "hdr" ? "heic" : format == "tiff" ? "tif" : format
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PicsRoom-\(UUID().uuidString.prefix(8)).\(ext)")
        do {
            if format == "pcraft" {
                _ = try await background { try request(["op":"save", "path":url.path]) }
            } else {
                let source=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString+".png")
                defer { try? FileManager.default.removeItem(at:source) }
                _ = try await background { try request(["op":"export", "path":source.path, "format":"png"]) }
                let state=nativeState
                let original=originalURL
                try await withCheckedThrowingContinuation { (continuation:CheckedContinuation<Void,Error>) in
                    renderQueue.async {
                        do { try DevelopRenderer.shared.export(original ?? source,to:url,state:state,format:format,original:original != nil,options:options); continuation.resume() }
                        catch { continuation.resume(throwing:error) }
                    }
                }
            }
            if present {shareURL = url};return url
        } catch { report(error);return nil }
    }
}

private struct SettingKey: CodingKey {
    var stringValue:String
    var intValue:Int? { nil }
    init?(stringValue:String) { self.stringValue=stringValue }
    init?(intValue:Int) { return nil }
    init(_ name:String) { stringValue=name }
}
extension Development {
    init(from decoder:Decoder) throws {
        self.init()
        let c=try decoder.container(keyedBy:SettingKey.self)
        exposure=try c.decodeIfPresent(Double.self,forKey:SettingKey("exposure")) ?? exposure
        brightness=try c.decodeIfPresent(Double.self,forKey:SettingKey("brightness")) ?? brightness
        contrast=try c.decodeIfPresent(Double.self,forKey:SettingKey("contrast")) ?? contrast
        saturation=try c.decodeIfPresent(Double.self,forKey:SettingKey("saturation")) ?? saturation
        vibrance=try c.decodeIfPresent(Double.self,forKey:SettingKey("vibrance")) ?? vibrance
        warmth=try c.decodeIfPresent(Double.self,forKey:SettingKey("warmth")) ?? warmth
        tint=try c.decodeIfPresent(Double.self,forKey:SettingKey("tint")) ?? tint
        lens=try c.decodeIfPresent(Double.self,forKey:SettingKey("lens")) ?? lens
        redFringe=try c.decodeIfPresent(Double.self,forKey:SettingKey("redFringe")) ?? redFringe
        blueFringe=try c.decodeIfPresent(Double.self,forKey:SettingKey("blueFringe")) ?? blueFringe
        mix=try c.decodeIfPresent([ColorBand].self,forKey:SettingKey("mix")) ?? mix
        highlights=try c.decodeIfPresent(Double.self,forKey:SettingKey("highlights")) ?? highlights
        shadows=try c.decodeIfPresent(Double.self,forKey:SettingKey("shadows")) ?? shadows
        whites=try c.decodeIfPresent(Double.self,forKey:SettingKey("whites")) ?? whites
        blacks=try c.decodeIfPresent(Double.self,forKey:SettingKey("blacks")) ?? blacks
        texture=try c.decodeIfPresent(Double.self,forKey:SettingKey("texture")) ?? texture
        clarity=try c.decodeIfPresent(Double.self,forKey:SettingKey("clarity")) ?? clarity
        dehaze=try c.decodeIfPresent(Double.self,forKey:SettingKey("dehaze")) ?? dehaze
        vignette=try c.decodeIfPresent(Double.self,forKey:SettingKey("vignette")) ?? vignette
        vignetteMid=try c.decodeIfPresent(Double.self,forKey:SettingKey("vignetteMid")) ?? vignetteMid
        grain=try c.decodeIfPresent(Double.self,forKey:SettingKey("grain")) ?? grain
        sharpness=try c.decodeIfPresent(Double.self,forKey:SettingKey("sharpness")) ?? sharpness
        sharpRadius=try c.decodeIfPresent(Double.self,forKey:SettingKey("sharpRadius")) ?? sharpRadius
        sharpDetail=try c.decodeIfPresent(Double.self,forKey:SettingKey("sharpDetail")) ?? sharpDetail
        sharpMask=try c.decodeIfPresent(Double.self,forKey:SettingKey("sharpMask")) ?? sharpMask
        noise=try c.decodeIfPresent(Double.self,forKey:SettingKey("noise")) ?? noise
        noiseDetail=try c.decodeIfPresent(Double.self,forKey:SettingKey("noiseDetail")) ?? noiseDetail
        noiseContrast=try c.decodeIfPresent(Double.self,forKey:SettingKey("noiseContrast")) ?? noiseContrast
        colorNoise=try c.decodeIfPresent(Double.self,forKey:SettingKey("colorNoise")) ?? colorNoise
        colorDetail=try c.decodeIfPresent(Double.self,forKey:SettingKey("colorDetail")) ?? colorDetail
        colorSmooth=try c.decodeIfPresent(Double.self,forKey:SettingKey("colorSmooth")) ?? colorSmooth
        blur=try c.decodeIfPresent(Double.self,forKey:SettingKey("blur")) ?? blur
        hue=try c.decodeIfPresent(Double.self,forKey:SettingKey("hue")) ?? hue
        fade=try c.decodeIfPresent(Double.self,forKey:SettingKey("fade")) ?? fade
        gradeWarm=try c.decodeIfPresent(Double.self,forKey:SettingKey("gradeWarm")) ?? gradeWarm
        gradeCool=try c.decodeIfPresent(Double.self,forKey:SettingKey("gradeCool")) ?? gradeCool
        curveShadows=try c.decodeIfPresent(Double.self,forKey:SettingKey("curveShadows")) ?? curveShadows
        curveMid=try c.decodeIfPresent(Double.self,forKey:SettingKey("curveMid")) ?? curveMid
        curveHighlights=try c.decodeIfPresent(Double.self,forKey:SettingKey("curveHighlights")) ?? curveHighlights
        if mix.count != 8 { mix=Array(repeating:ColorBand(),count:8) }
    }
}

extension EditorModel {
    func loadCatalog() {
        do {
            let url=documentsURL.appendingPathComponent("Studio-Katalog.json")
            if FileManager.default.fileExists(atPath:url.path) { catalog=try JSONDecoder().decode(PhotoCatalog.self,from:Data(contentsOf:url)) }
        } catch { self.error="Katalog konnte nicht gelesen werden: \(error.localizedDescription)" }
    }
    private func saveCatalog() {
        do { try JSONEncoder().encode(catalog).write(to:documentsURL.appendingPathComponent("Studio-Katalog.json"),options:.atomic) }
        catch { self.error="Katalog konnte nicht gespeichert werden: \(error.localizedDescription)" }
    }
    func organize(_ urls:Set<URL>,album:String?=nil,rating:Int?=nil) {
        for url in urls {
            var entry=catalog.entries[url.lastPathComponent] ?? CatalogEntry()
            if let album { entry.album=album.trimmingCharacters(in:.whitespacesAndNewlines) }
            if let rating { entry.rating=min(5,max(0,rating)) }
            catalog.entries[url.lastPathComponent]=entry
        };saveCatalog()
    }
    func saveLook(_ name:String) {
        let clean=name.trimmingCharacters(in:.whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        catalog.presets.append(UserLook(name:clean,development:development,curves:advanced.curves,grades:advanced.grades,balance:advanced.balance));saveCatalog()
    }
    func importLook(_ url:URL) {
        let access=url.startAccessingSecurityScopedResource();defer {if access {url.stopAccessingSecurityScopedResource()}}
        do {
            guard (try url.resourceValues(forKeys:[.fileSizeKey]).fileSize ?? 0) <= 4_000_000 else {throw EditorError(message:"Preset ist zu groß (max. 4 MB).")}
            let data=try Data(contentsOf:url)
            var preset:UserLook
            if url.pathExtension.lowercased() == "xmp" { preset=try PresetXML.read(data,name:url.deletingPathExtension().lastPathComponent) }
            else if ["picsroompreset","json"].contains(url.pathExtension.lowercased()) {preset=try JSONDecoder().decode(UserLook.self,from:data)}
            else {throw EditorError(message:"Bitte eine XMP-Datei aus Lightroom oder ein PicsRoom-Preset auswählen. ZIP-Dateien vorher entpacken.")}
            guard preset.curves.count==4,preset.curves.allSatisfy({ points in points.count==5 && points.allSatisfy({(0...1).contains($0.x) && (0...1).contains($0.y)}) && zip(points,points.dropFirst()).allSatisfy({$0.x < $1.x}) }),preset.grades.count==3,preset.development.mix.count==8 else {throw EditorError(message:"Ungültiges Preset.")}
            preset.id=UUID().uuidString;catalog.presets.append(preset);saveCatalog()
            status="Preset importiert: \(preset.name)"
            if preset.importedValues != nil {error="Lightroom-Preset importiert: \(preset.importedValues?.count ?? 0) Licht- und Farbwerte. Die Darstellung ist angenähert. Profile, Kurven, HSL, Masken, Weißabgleich und andere Adobe-Effekte werden derzeit nicht übernommen."}
        } catch {self.error=error.localizedDescription}
    }
    func exportSelection(_ urls:[URL],options:ExportOptions) async throws -> [URL] {
        guard !busy,!batchRunning,!urls.isEmpty,let restore=projectURL else {throw EditorError(message:"Bitte warten, bis die Bearbeitung gespeichert ist.")}
        batchRunning=true;error=nil
        defer {batchRunning=false}
        var outputs:[URL]=[]
        do {
            for (index,url) in urls.enumerated() {
                await openProject(url)
                guard projectURL==url,error==nil else {throw EditorError(message:error ?? "Foto konnte nicht geöffnet werden.")}
                status="Export \(index+1) / \(urls.count)"
                let output:URL
                if options.format == "original" {
                    guard let originalURL else {throw EditorError(message:"Für dieses ältere Projekt ist kein Original gespeichert.")}
                    output=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString+"."+originalURL.pathExtension)
                    try await background {try FileManager.default.copyItem(at:originalURL,to:output)}
                } else {
                    guard let generated=await export(options.format,options:options,present:false) else {throw EditorError(message:error ?? "Export fehlgeschlagen.")};output=generated
                }
                let cleaned=options.filename.unicodeScalars.filter {CharacterSet.alphanumerics.contains($0) || "-_ ".unicodeScalars.contains($0)}.map(String.init).joined()
                let name=String(cleaned.trimmingCharacters(in:.whitespaces).prefix(60))
                let folder=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString,isDirectory:true)
                try FileManager.default.createDirectory(at:folder,withIntermediateDirectories:true)
                let named=folder.appendingPathComponent("\(name.isEmpty ? "PicsRoom" : name)-\(index+1).\(output.pathExtension)")
                try FileManager.default.moveItem(at:output,to:named);outputs.append(named)
            }
            await openProject(restore);status="\(outputs.count) Datei(en) exportiert"
            return outputs
        } catch {
            let failure=error
            for url in outputs {try? FileManager.default.removeItem(at:url.deletingLastPathComponent())}
            await openProject(restore)
            throw failure
        }
    }
    func exportLook(_ preset:UserLook) {
        do {let url=FileManager.default.temporaryDirectory.appendingPathComponent("PicsRoom-\(UUID().uuidString.prefix(8)).picsroompreset");try JSONEncoder().encode(preset).write(to:url,options:.atomic);shareURL=url}
        catch {self.error=error.localizedDescription}
    }
    func deleteLook(_ id:String) { catalog.presets.removeAll {$0.id==id};saveCatalog() }
    func applyLook(_ preset:UserLook) async {
        if let values=preset.importedValues {
            for (key,value) in values { if let path=PresetXML.fields[key]?.0 { development[keyPath:path]=min(PresetXML.fields[key]!.1.upperBound,max(PresetXML.fields[key]!.1.lowerBound,value)) } }
        } else {
            development=preset.development;advanced.curves=preset.curves;advanced.grades=preset.grades;advanced.balance=preset.balance
        }
        previewChanges();await commitDevelopment()
    }
    func batchApply(_ urls:Set<URL>) async {
        guard !busy,!batchRunning,let restore=projectURL else { return }
        let preset=UserLook(name:"",development:development,curves:advanced.curves,grades:advanced.grades,balance:advanced.balance)
        batchRunning=true;defer { batchRunning=false }
        var completed=0
        for url in urls.sorted(by:{$0.path<$1.path}) {
            await openProject(url)
            guard projectURL==url,error == nil else { break }
            await applyLook(preset)
            guard error == nil else { break }
            completed+=1
        }
        let failure=error;await openProject(restore);error=failure
        status="Einstellungen auf \(completed) von \(urls.count) Fotos angewendet"
    }
    func rangeMask(color:Bool,center:Double,tolerance:Double) async {
        guard !busy,let base=basePreview,masks.count<12 else { return }
        busy=true
        do {
            let data:Data=try await withCheckedThrowingContinuation { continuation in
                renderQueue.async {
                    do {
                        let source=CIImage(cgImage:base)
                        let kernel=CIColorKernel(source:"""
                        kernel vec4 range(__sample p,float center,float tolerance,float color) {
                            float value=dot(p.rgb,vec3(0.2126,0.7152,0.0722));float chroma=1.0;
                            if(color>0.5) {
                                float mx=max(p.r,max(p.g,p.b)),mn=min(p.r,min(p.g,p.b)),d=mx-mn;float h=0.0;
                                if(d>0.0001){if(mx==p.r)h=mod((p.g-p.b)/d,6.0);else if(mx==p.g)h=(p.b-p.r)/d+2.0;else h=(p.r-p.g)/d+4.0;}
                                value=fract(h/6.0+1.0);chroma=smoothstep(0.01,0.08,d);
                            }
                            float distance=abs(value-center);if(color>0.5)distance=min(distance,1.0-distance);
                            float m=(1.0-smoothstep(tolerance*0.6,max(0.001,tolerance),distance))*chroma;return vec4(m,m,m,1.0);
                        }
                        """)
                        guard let result=kernel?.apply(extent:source.extent,arguments:[source,center,tolerance,color ? 1.0 : 0.0]),let cg=DevelopRenderer.shared.context.createCGImage(result,from:result.extent),let png=UIImage(cgImage:cg).pngData() else { throw EditorError(message:"Bereichsauswahl fehlgeschlagen.") }
                        continuation.resume(returning:png)
                    } catch { continuation.resume(throwing:error) }
                }
            }
            var m=LocalMask();m.kind="ai";m.name=color ? "Farbbereich" : "Helligkeitsbereich";m.rasterPNG=data;m.rasterCorners=[MaskPoint(x:0,y:0),MaskPoint(x:1,y:0),MaskPoint(x:0,y:1)]
            masks.append(m);selectedMask=m.id;busy=false;previewChanges();await commitDevelopment()
        } catch { busy=false;report(error) }
    }
    func resetRAW() async {
        guard let originalURL,rawSettings != nil,!busy else { return }
        do {
            let settings=try await background { () throws -> RAWSettings in
                guard let settings=OriginalPipeline.rawSettings(originalURL) else { throw EditorError(message:"RAW-Weißabgleich konnte nicht gelesen werden.") };return settings
            }
            rawSettings=settings;previewChanges();await commitDevelopment()
        } catch { report(error) }
    }
    func exportOriginal() async {
        guard let originalURL,!busy else { return }
        do {
            let url=FileManager.default.temporaryDirectory.appendingPathComponent("Original-\(UUID().uuidString.prefix(8)).\(originalURL.pathExtension)")
            try await background { try FileManager.default.copyItem(at:originalURL,to:url) };shareURL=url
        } catch { report(error) }
    }
}

#if DEBUG
extension EditorModel {
    func verifyRAW() async {
        do {
            for name in ["canon-250d.cr3","proraw.dng"] {
                let url=documentsURL.appendingPathComponent(name)
                let input=try await background { try Data(contentsOf:url) }
                await importPhoto(input)
                if let error { throw EditorError(message:error) }
                guard rawSettings != nil,let project=projectURL,let original=originalURL,let base=basePreview else { throw EditorError(message:"RAW wurde nicht als Original erkannt: \(name)") }
                guard try Data(contentsOf:original)==input else { throw EditorError(message:"Originalbytes verändert") }
                let before=try await render(base,nativeState)
                rawSettings!.temperature=3200;development.exposure = -0.8
                await commitDevelopment()
                let expected=nativeState
                let after=try await render(base,expected)
                guard before.pngData() != after.pngData() else { throw EditorError(message:"RAW-Regler ohne Wirkung") }
                await openProject(project)
                guard nativeState == expected,let reopened=originalURL,try Data(contentsOf:reopened)==input else { throw EditorError(message:"RAW-Projekt-Roundtrip fehlgeschlagen") }
                let width=document!.width,height=document!.height
                await command("image.imageRotation.90cw")
                guard document?.width==height,document?.height==width,sourceEdits?.last?.kind=="image.imageRotation.90cw" else { throw EditorError(message:"RAW-Drehung nicht gespeichert") }
                await command("edit.undo")
                guard nativeState==expected else { throw EditorError(message:"RAW-Undo fehlgeschlagen") }
                await export("tiff")
                guard let tiff=shareURL,let source=CGImageSourceCreateWithURL(tiff as CFURL,nil),let properties=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],(properties[kCGImagePropertyDepth] as? Int)==16 else { throw EditorError(message:"16-Bit-Export fehlgeschlagen: \(error ?? "")") }
                shareURL=nil
                await export("pcraft")
                guard let shared=shareURL else { throw EditorError(message:"Projektweitergabe fehlgeschlagen") }
                shareURL=nil;await importFile(shared)
                guard nativeState==expected,let packed=originalURL,try Data(contentsOf:packed)==input else { throw EditorError(message:"Geteiltes RAW-Projekt unvollständig") }
            }
            try Data("passed".utf8).write(to:documentsURL.appendingPathComponent("raw-verification.txt"))
            status="RAW geprüft: 2 Originale"
        } catch { report(error) }
    }
    func verifyExport() async {
        do {
            guard let first=projectURL,let original=originalURL else {throw EditorError(message:"Export-Testquelle fehlt")}
            let originalData=try Data(contentsOf:original)
            var settings=ExportOptions();settings.maxSide=512;settings.quality=1
            guard let full=await export("jpg",options:settings,present:false) else {throw EditorError(message:"JPEG fehlgeschlagen")}
            let fullData=try Data(contentsOf:full)
            settings.quality=0.6
            guard let compressed=await export("jpg",options:settings,present:false) else {throw EditorError(message:"JPEG-Qualität fehlgeschlagen")}
            guard try Data(contentsOf:compressed).count < fullData.count else {throw EditorError(message:"Qualitätsstufen ohne Wirkung")}
            settings.format="png"
            guard let plain=await export("png",options:settings,present:false) else {throw EditorError(message:"PNG fehlgeschlagen")}
            settings.watermark="PicsRoom Test"
            guard let marked=await export("png",options:settings,present:false) else {throw EditorError(message:"Wasserzeichen fehlgeschlagen")}
            guard try Data(contentsOf:plain) != Data(contentsOf:marked) else {throw EditorError(message:"Wasserzeichen ohne Wirkung")}
            await loadDemo();guard let second=projectURL else {throw EditorError(message:"Zweites Projekt fehlt")}
            settings.filename="Meine Bilder"
            let batch=try await exportSelection([first,second],options:settings)
            guard batch.count==2,projectURL==second else {throw EditorError(message:"Stapelexport oder Wiederherstellung fehlgeschlagen")}
            for file in batch {
                guard let source=CGImageSourceCreateWithURL(file as CFURL,nil),let p=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],let w=p[kCGImagePropertyPixelWidth] as? Int,let h=p[kCGImagePropertyPixelHeight] as? Int,max(w,h)==512,file.lastPathComponent.hasPrefix("Meine Bilder-") else {throw EditorError(message:"Exportgröße oder Name falsch")}
            }
            settings.format="original"
            let originals=try await exportSelection([first],options:settings)
            guard try Data(contentsOf:originals[0])==originalData else {throw EditorError(message:"Original verändert")}
            settings.watermark="";settings.colorSpace="P3"
            for format in ["tiff","hdr"] {
                guard let file=await export(format,options:settings,present:false),let source=CGImageSourceCreateWithURL(file as CFURL,nil),let p=CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],p[kCGImagePropertyDepth] as? Int == (format == "tiff" ? 16 : 10) else {throw EditorError(message:"Farbtiefe falsch")}
            }
            status="Exportoptionen geprüft"
        } catch {report(error)}
    }
    func verifyPresets() async {
        do {
            let xml="""
            <x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="1.25" crs:Contrast2012="28"><crs:Vibrance>19</crs:Vibrance></rdf:Description></rdf:RDF></x:xmpmeta>
            """
            let preset=try PresetXML.read(Data(xml.utf8),name:"Test")
            guard preset.importedValues?.count==3 else {throw EditorError(message:"XMP-Felder fehlen")}
            development.shadows=37
            await applyLook(preset)
            guard development.exposure==1.25,development.contrast==28,development.vibrance==19,development.shadows==37 else {throw EditorError(message:"XMP-Werte oder bestehende Werte verloren")}
            let decoded=try JSONDecoder().decode(UserLook.self,from:JSONEncoder().encode(preset))
            guard decoded.importedValues==preset.importedValues else {throw EditorError(message:"Preset-Roundtrip fehlgeschlagen")}
            do { _=try PresetXML.read(Data("<broken>".utf8),name:"broken");throw EditorError(message:"Ungültiges XML akzeptiert") }
            catch let failure as EditorError { if failure.message=="Ungültiges XML akzeptiert" {throw failure} }
            guard let first=projectURL else {throw EditorError(message:"Projekt fehlt")}
            await loadDemo()
            guard let second=projectURL,first != second else {throw EditorError(message:"Zweites Projekt fehlt")}
            await openSelection([first,second]);await moveSelection(1)
            guard projectURL==second,selectionIndex==1 else {throw EditorError(message:"Mehrfachauswahl fehlgeschlagen")}
            await moveSelection(-1)
            guard projectURL==first,development.exposure==1.25 else {throw EditorError(message:"Auswahl verliert Bearbeitung")}
            status="Presets und Auswahl geprüft"
        } catch {report(error)}
    }
    func verifyAdvanced() async {
        do {
            guard let base=basePreview else { throw EditorError(message:"Testbasis fehlt") }
            let neutral=try await render(base,nativeState)
            var variants:[NativeState]=[]
            var curve=nativeState;curve.advanced!.curves[1][2].y=0.8;variants.append(curve)
            var grade=nativeState;grade.advanced!.grades[1]=Grade(hue:200,saturation:70,luminance:10);variants.append(grade)
            var perspective=nativeState;perspective.advanced!.vertical=30;perspective.advanced!.angle=3;variants.append(perspective)
            var repair=nativeState;repair.advanced!.repairs=[RepairSpot(source:MaskPoint(x:0.1,y:0.8),destination:MaskPoint(x:0.7,y:0.25),radius:0.1,heal:false)];variants.append(repair)
            var healed=repair;healed.advanced!.repairs[0].heal=true;variants.append(healed)
            for (i,state) in variants.enumerated() {
                do { let result=try await render(base,state);guard result.pngData() != neutral.pngData() else { throw EditorError(message:"Filter ohne Wirkung") } }
                catch { throw EditorError(message:"Erweiterung \(i): \(error.localizedDescription)") }
            }
            if let original=originalURL {
                let before=try OriginalPipeline.image(original,state:repair,maxSide:1600)
                let e=before.extent
                let cropped=OriginalPipeline.origin(before.cropped(to:CGRect(x:e.width*0.5,y:0,width:e.width*0.5,height:e.height)))
                var after=repair
                after.sourceEdits=[SourceEdit(kind:"image.crop",x:0.5,y:0,width:0.5,height:1)]
                after.advanced!.repairs[0].source.x=(repair.advanced!.repairs[0].source.x-0.5)*2
                after.advanced!.repairs[0].destination.x=(repair.advanced!.repairs[0].destination.x-0.5)*2
                after.advanced!.repairs[0].radius *= 2
                let actual=try OriginalPipeline.image(original,state:after,maxSide:1600)
                let context=DevelopRenderer.shared.context
                guard let a=context.createCGImage(cropped,from:cropped.extent),let b=context.createCGImage(actual,from:actual.extent),UIImage(cgImage:a).pngData()==UIImage(cgImage:b).pngData() else { throw EditorError(message:"Klonquelle außerhalb des Zuschnitts ging verloren") }
            }
            advanced=grade.advanced!;await commitDevelopment()
            let expected=nativeState
            guard let project=projectURL else { throw EditorError(message:"Projekt fehlt") }
            await openProject(project);guard nativeState==expected else { throw EditorError(message:"Grading nicht wiederhergestellt") }
            await rangeMask(color:false,center:0.5,tolerance:0.3)
            guard masks.last?.rasterPNG != nil else { throw EditorError(message:"Bereichsmaske fehlt") }
            let count=catalog.presets.count;saveLook("Prüfung");loadCatalog();guard catalog.presets.count==count+1 else { throw EditorError(message:"Look nicht gespeichert") }
            if let id=catalog.presets.last?.id { deleteLook(id) }
            organize([project],album:"Prüfung",rating:4);loadCatalog();guard catalog.entries[project.lastPathComponent]?.rating==4 else { throw EditorError(message:"Katalog nicht gespeichert") }
            await loadDemo();guard let second=projectURL else { throw EditorError(message:"Zweites Testprojekt fehlt") }
            development.contrast=21;advanced.grades[0].saturation=20;await commitDevelopment()
            await batchApply([project,second]);await openProject(project)
            guard development.contrast==21,advanced.grades[0].saturation==20,!masks.isEmpty else { throw EditorError(message:"Stapelbearbeitung verlor Einstellungen oder Masken") }
            await openProject(second)
            guard development.contrast==21 else { throw EditorError(message:"Stapelbearbeitung zweites Foto fehlgeschlagen") }
            await export("hdr")
            guard let hdr=shareURL,let src=CGImageSourceCreateWithURL(hdr as CFURL,nil),let props=CGImageSourceCopyPropertiesAtIndex(src,0,nil) as? [CFString:Any],(props[kCGImagePropertyDepth] as? Int)==10 else { throw EditorError(message:"HDR-Export: \(error ?? "Keine 10-Bit-Datei")") }
            shareURL=nil
            status="Erweiterungen geprüft"
        } catch { report(error) }
    }
}
#endif
