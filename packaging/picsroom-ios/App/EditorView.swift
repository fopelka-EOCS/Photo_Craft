import SwiftUI
import PhotosUI
import UniformTypeIdentifiers

private enum StudioTool: String, CaseIterable {
    case light = "Licht", color = "Farbe", looks = "Looks", crop = "Zuschnitt", layers = "Ebenen", effects = "Effekte", details = "Details", masks = "Masken", curve = "Kurve", optics = "Optik"
    static var allCases: [StudioTool] { [.light,.color,.effects,.details,.masks,.curve,.optics,.looks,.crop,.layers] }
    var symbol: String {
        switch self {
        case .light: "sun.max"
        case .color: "thermometer.medium"
        case .looks: "camera.filters"
        case .crop: "crop.rotate"
        case .layers: "square.3.layers.3d"
        case .effects: "square.on.circle"
        case .details: "triangle.fill"
        case .masks: "circle.dashed"
        case .curve: "point.topleft.down.to.point.bottomright.curvepath"
        case .optics: "camera.aperture"
        }
    }
}
struct EditorView: View {
    @ObservedObject var model: EditorModel
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @State private var photo: [PhotosPickerItem] = []
    @State private var importingPreset = false
    @State private var tool: StudioTool = .light
    @State private var studioSection = "Bearbeiten"
    private enum ImportSource { case photos, files }
    @State private var pendingImport: ImportSource?
    @State private var pickingPhoto = false
    @State private var files = false
    @State private var library = false
    @State private var sharing = false
    @State private var referenceLook=false
    @State private var exportDialog=false
    @State private var controls = true
    @GestureState private var panelDrag: CGFloat = 0
    @Environment(\.scenePhase) private var scenePhase
    @State private var dragging: String?
    @State private var comparing = false
    @State private var comparisonObserved = false
    @State private var cropRatio:Double?
    @State private var cropName="Frei"
    @State private var cropTab="Format"
    @State private var aiExpansion=false
    @State private var cropRect = CGRect(x:0.08,y:0.08,width:0.84,height:0.84)
    @State private var loadingPhoto = false
    @State private var showReset = false
    @State private var samplingWhiteBalance = false
    @State private var colorTab = "Farbe"
    @State private var colorBand = 0
    @State private var detailTab = "Schärfen"
    @State private var effectTab = "Effekte"
    @State private var brushRadius = 0.04
    @State private var erasing = false
    @State private var maskOverlay = true
    @State private var pencilOnly = false
    @State private var curveChannel=0
    @State private var gradingZone=0
    @State private var presetName=""
    @State private var selectedProjects:Set<URL>=[]
    @State private var albumName=""
    @State private var albumFilter="Alle"
    @State private var minimumRating=0
    @State private var rangeColor=false
    @State private var rangeCenter=0.5
    @State private var rangeTolerance=0.2
    @State private var repairMode=false
    @State private var repairSource:MaskPoint?
    @State private var repairRadius=0.04
    @State private var repairHeal=true

    var body: some View {
        GeometryReader { geometry in
            let sidePanel=geometry.size.width>=760
            let panelWidth=min(400,max(320,geometry.size.width*0.34))
            let panelHeight=sidePanel ? max(90,geometry.size.height-64-32-(studioSection == "Bearbeiten" ? 137 : 71)-16) : min(175,max(115,geometry.size.height*0.19))
            ZStack {
                Color(red:0.035,green:0.043,blue:0.05).ignoresSafeArea()
                if let image = comparing ? (model.originalPreview ?? model.preview) : model.preview {
                    ZoomCanvas(image:image,onCompare:comparePhoto)
                        .id(studioSection == "Zuschnitt")
                        .allowsHitTesting(!controls || (studioSection != "Zuschnitt" && studioSection != "Maskieren" && !samplingWhiteBalance && !repairMode))
                        .padding(canvasInsets(geometry.size))
                    if repairMode && controls,let doc=model.document {
                        WhiteBalanceSampler(imageSize:CGSize(width:doc.width,height:doc.height),instruction:repairSource == nil ? "Zuerst auf die saubere Quellstelle tippen" : "Jetzt auf die zu korrigierende Stelle tippen") { point in
                            if let source=repairSource {
                                model.advanced.repairs.append(RepairSpot(source:source,destination:point,radius:repairRadius,heal:repairHeal));repairSource=nil
                                model.previewChanges();Task { await model.commitDevelopment() }
                            } else { repairSource=point }
                        }.padding(canvasInsets(geometry.size))
                    }
                    if samplingWhiteBalance, let doc=model.document {
                        WhiteBalanceSampler(imageSize:CGSize(width:doc.width,height:doc.height)) { point in
                            samplingWhiteBalance=false; Task { await model.sampleWhiteBalance(point) }
                        }.padding(canvasInsets(geometry.size))
                    }
                    if studioSection == "Maskieren" && controls, let doc=model.document,
                       let mask=model.masks.first(where:{$0.id == model.selectedMask}) {
                        MaskEditor(mask:maskBinding(mask),imageSize:CGSize(width:doc.width,height:doc.height),brushRadius:brushRadius,erase:erasing,overlay:maskOverlay,pencilOnly:pencilOnly,changed:{maskOverlay=true;model.previewChanges()},finished:{Task { await model.commitDevelopment() }})
                            .id(mask.id).padding(canvasInsets(geometry.size))
                    }
                    if studioSection == "Zuschnitt" && cropTab == "Format" && controls, let doc = model.document {
                        CropOverlay(rect:$cropRect, imageSize:CGSize(width:doc.width,height:doc.height),ratio:cropRatio)
                            .padding(canvasInsets(geometry.size))
                    }
                } else { welcome }
                if model.document != nil && comparing {
                    Text("VORHER").font(.caption2.weight(.semibold)).tracking(1.5)
                        .padding(12).background(.black.opacity(0.55),in:Capsule()).allowsHitTesting(false).accessibilityIdentifier("before-indicator")
                }
                VStack(spacing:0) {
                    header.opacity(dragging == nil ? 1 : 0.15)
                    if model.editingProjects.count > 1 {
                        HStack {
                            Button { Task { await model.moveSelection(-1) } } label: { Image(systemName:"chevron.left") }.accessibilityLabel("Vorheriges Foto").disabled(model.selectionIndex == 0)
                            Text("\(model.selectionIndex+1) / \(model.editingProjects.count)").monospacedDigit()
                            Button { Task { await model.moveSelection(1) } } label: { Image(systemName:"chevron.right") }.accessibilityLabel("Nächstes Foto").disabled(model.selectionIndex+1 >= model.editingProjects.count)
                        }.padding(8).background(.black.opacity(0.65),in:Capsule()).disabled(locked)
                    }
                    Spacer(minLength:0)
                    if model.document != nil && controls {
                        VStack(spacing:0) {
                            panelHandle
                            sectionControls.opacity(comparing ? 0 : 1).allowsHitTesting(!comparing)
                            dock
                        }
                        .frame(width:sidePanel ? panelWidth : nil)
                        .frame(maxWidth:sidePanel ? nil : 640)
                        .clipShape(RoundedRectangle(cornerRadius:sidePanel ? 14 : 0))
                        .frame(maxWidth:.infinity,alignment:sidePanel ? .trailing : .center)
                        .padding(.trailing,sidePanel ? 8 : 0).padding(.bottom,8)
                        .offset(y:panelDrag).transition(.move(edge:.bottom))
                    } else if model.document != nil {
                        panelHandle
                    }
                }
                if model.busy || loadingPhoto {
                    VStack { Spacer(); HStack(spacing:8) {
                        ProgressView().tint(.white)
                        Text(loadingPhoto ? "Foto laden" : model.segmenting ? "Bereich wird erkannt …" : "Wird gespeichert").font(.caption)
                    }.padding(12).background(.black.opacity(0.75),in:Capsule()).padding(.bottom,controls ? 320 : 24) }
                    .allowsHitTesting(false)
                }
            }
            .environment(\.studioPanelHeight,panelHeight)
            .foregroundStyle(.white)
            .animation(.easeOut(duration:0.18),value:dragging)
            .animation(.easeOut(duration:0.18),value:controls)
            .sheet(isPresented:$aiExpansion) { AIExpansionView(model:model,initialRatio:cropRatio) }
            .sheet(isPresented:$referenceLook){ReferenceLookView(model:model)}
        .sheet(isPresented:$exportDialog) { ExportView(model:model) }
            .sheet(isPresented:$library,onDismiss:presentPendingImport) { projectLibrary }
            .photosPicker(isPresented:$pickingPhoto,selection:$photo,maxSelectionCount:30,selectionBehavior:.ordered,matching:.images,preferredItemEncoding:.current)
            .sheet(isPresented:$sharing,onDismiss:{ model.shareURL=nil }) {
                if let url=model.shareURL { ShareSheet(url:url) }
            }
            .fileImporter(isPresented:$files,allowedContentTypes:importingPreset ? [.data,.xml,.json] : [.image,.rawImage,UTType(filenameExtension:"cr3") ?? .data,UTType(filenameExtension:"pcraft") ?? .data]) { result in
                switch result {
                case .success(let url): Task { if importingPreset { model.importLook(url) } else { let previous=model.projectURL;await model.importFile(url);if model.projectURL != previous {model.editingProjects=[]} } }
                case .failure(let error): model.error=error.localizedDescription
                }
            }
            .onChange(of:controls) { _,_ in comparing=false; samplingWhiteBalance=false;updateSpatialEditing() }
            .onChange(of:tool) { _,_ in comparing=false }
            .onChange(of:studioSection) { _,_ in comparing=false;samplingWhiteBalance=false;repairMode=false;repairSource=nil;updateSpatialEditing() }
            .onChange(of:cropTab) {_,_ in updateSpatialEditing()}
            .onChange(of:repairMode) { _,_ in updateSpatialEditing() }
            .onChange(of:scenePhase) { _,phase in if phase != .active { comparing=false } }
            .onChange(of:model.shareURL) { _,value in sharing=value != nil }
            .onChange(of:model.selectedMask) { _,_ in maskOverlay=true }
            .onChange(of:photo) { _,items in
                guard !items.isEmpty else { return }
                Task {
                    loadingPhoto=true
                    defer { loadingPhoto=false; photo=[] }
                    var imported:[URL]=[]
                    for item in items {
                        do {
                            let previous=model.projectURL
                            guard let data=try await item.loadTransferable(type:Data.self) else { continue }
                            await model.importPhoto(data)
                            if let url=model.projectURL,url != previous { imported.append(url) }
                        } catch { model.error=error.localizedDescription }
                    }
                    if !imported.isEmpty { await model.openSelection(imported) }
                }
            }
            .alert("PicsRoom",isPresented:Binding(get:{model.error != nil},set:{if !$0 {model.error=nil}})) {
                Button("OK") { model.error=nil }
            } message: { Text(model.error ?? "") }
            .confirmationDialog("Alle Licht- und Farbanpassungen zurücksetzen?",isPresented:$showReset,titleVisibility:.visible) {
                Button("Anpassungen zurücksetzen",role:.destructive) { Task { await model.applyPreset(Development()) } }
            }
        }
    }
    private func comparePhoto(_ held:Bool) {
        comparing=held
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--verify-gestures") {
            if held { comparisonObserved=true }
            else if comparisonObserved { model.status="Vergleich geprüft" }
        }
        #endif
    }
    private var panelHandle: some View {
        Button { controls.toggle() } label: {
            VStack(spacing:3) {
                Capsule().fill(.white.opacity(0.55)).frame(width:36,height:4)
                if !controls { Image(systemName:"chevron.up").font(.system(size:10,weight:.semibold)) }
            }.frame(maxWidth:.infinity).frame(height:controls ? 20:36).contentShape(Rectangle())
        }.buttonStyle(.plain).background(.black.opacity(controls ? 0.55 : 0.25))
            .accessibilityLabel(controls ? "Regler ausblenden" : "Regler einblenden")
            .accessibilityIdentifier("panel-handle")
            .highPriorityGesture(DragGesture(minimumDistance:10)
                .updating($panelDrag) { value,state,_ in if controls { state=max(0,value.translation.height) } }
                .onEnded { value in
                    guard abs(value.translation.height)>abs(value.translation.width) else { return }
                    if value.translation.height>30 { controls=false }
                    if value.translation.height < -30 { controls=true }
                })
    }
    private func canvasInsets(_ size:CGSize) -> EdgeInsets {
        guard controls else { return EdgeInsets() }
        if size.width>=760 { return EdgeInsets(top:64,leading:16,bottom:16,trailing:min(400,max(320,size.width*0.34))+24) }
        if studioSection == "Zuschnitt" || studioSection == "Maskieren" { return EdgeInsets(top:64,leading:22,bottom:min(175,max(115,size.height*0.19))+86,trailing:22) }
        return EdgeInsets()
    }
    private var locked: Bool { model.busy || loadingPhoto || model.batchRunning }
    private func glass<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        StudioPanel(opaque:reduceTransparency,content:content())
    }
    private func icon(_ title: String,_ symbol: String,action:@escaping () -> Void) -> some View {
        Button(action:action) { Image(systemName:symbol).font(.system(size:16,weight:.medium)).frame(width:44,height:44) }
            .accessibilityLabel(title)
    }
    private var header: some View {
        HStack(spacing:2) {
            icon("Mediathek","square.grid.2x2") { model.refreshProjects(); library=true }.disabled(locked)
            VStack(alignment:.leading,spacing:3) {
                Text(model.rawSettings == nil ? "PicsRoom" : "PicsRoom · RAW").font(.system(size:15,weight:.semibold,design:.rounded))
                if let doc=model.document {
                    Text(verbatim:"\(doc.width) × \(doc.height)  ·  \(model.status)")
                        .font(.system(size:9,weight:.medium)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
                } else { Text("DEIN MOBILES FOTOSTUDIO").font(.system(size:8,weight:.medium)).tracking(1) }
            }
            Spacer(minLength:0)
            if model.document != nil {
                icon("Rückgängig","arrow.uturn.backward") { Task { await model.command("edit.undo") } }
                    .disabled(locked || !(model.document?.canUndo ?? false))
                icon("Bedienelemente ein-/ausblenden",controls ? "arrow.up.left.and.arrow.down.right" : "slider.horizontal.3") { controls.toggle() }
                icon("Exportieren und speichern","square.and.arrow.up") {exportDialog=true}.disabled(locked)
                Menu {
                    Button("Original teilen") {Task {await model.exportOriginal()}}
                    Divider()
                    Button("Anpassungen zurücksetzen") {showReset=true}
                    Button("Erneut speichern",systemImage:"internaldrive") { Task { await model.save() } }
                    Button("Wiederholen",systemImage:"arrow.uturn.forward") { Task { await model.command("edit.redo") } }
                        .disabled(!(model.document?.canRedo ?? false))
                } label: { Image(systemName:"ellipsis").frame(width:36,height:44) }
                    .disabled(locked).accessibilityLabel("Weitere Aktionen")
            }
        }
        .padding(.horizontal,8).padding(.vertical,4)
        .background(LinearGradient(colors:[.black.opacity(0.66),.black.opacity(0)],startPoint:.top,endPoint:.bottom))
    }
    private var welcome: some View {
        VStack(spacing:24) {
            ZStack {
                RoundedRectangle(cornerRadius:28).fill(LinearGradient(colors:[.white.opacity(0.12),.white.opacity(0.02)],startPoint:.topLeading,endPoint:.bottomTrailing)).frame(width:100,height:100)
                Image(systemName:"camera.aperture").font(.system(size:48,weight:.ultraLight))
            }
            VStack(spacing:10) {
                Text("Raum für dein Bild.").font(.system(size:30,weight:.light,design:.rounded))
                Text("Licht formen. Farben entdecken.\nGanz in deinem Moment bleiben.").font(.subheadline).foregroundStyle(.white.opacity(0.55)).multilineTextAlignment(.center)
            }
            Button { pickingPhoto=true } label: {
                Label("Foto öffnen",systemImage:"plus").font(.subheadline.weight(.semibold)).padding(.horizontal,28).padding(.vertical,16)
                    .background(.white,in:Capsule()).foregroundStyle(.black)
            }.disabled(locked)
            Button("Aus Dateien importieren") { importingPreset=false;files=true }.font(.footnote).foregroundStyle(.white.opacity(0.7)).disabled(locked)
            Text("OFFLINE  ·  VOLLE AUFLÖSUNG BIS 50 MP").font(.system(size:9,weight:.medium)).tracking(1.5).foregroundStyle(.white.opacity(0.35))
        }.padding(24)
    }
    @ViewBuilder private var sectionControls: some View {
        if studioSection == "Aktionen" || studioSection == "Retusche" {
            glass {
                VStack(alignment:.leading,spacing:18) {
                    Text(studioSection == "Retusche" ? "Porträt gezielt bearbeiten" : "Bereich automatisch auswählen").font(.headline)
                    automaticChoices(studioSection == "Retusche" ? [.skin,.hair,.eyes] : [.subject,.background,.sky,.objects])
                    if studioSection == "Retusche" {
                        Toggle("Reparieren / Klonen",isOn:$repairMode)
                        if repairMode {
                            Toggle("Helligkeit angleichen",isOn:$repairHeal)
                            Slider(value:$repairRadius,in:0.005...0.15) { Text("Größe") }
                            Text("Quelle und Ziel antippen. Für größere Objekte mehrere Stellen korrigieren.").font(.caption)
                            Button("Letzte Korrektur entfernen") { if !model.advanced.repairs.isEmpty { model.advanced.repairs.removeLast();advancedChanged();advancedFinished() } }.disabled(model.advanced.repairs.isEmpty)
                        }
                    }
                    Text("Erkennung auf deinem iPhone. Anschließend Auswahl prüfen und mit dem Pinsel verfeinern.").font(.caption).foregroundStyle(.secondary)
                }
            }
        } else { toolControls }
    }
    private func automaticChoices(_ regions:[AutomaticRegion]) -> some View {
        ScrollView(.horizontal,showsIndicators:false) { HStack(spacing:12) {
            ForEach(regions) { region in
                Button {
                    tool = .masks;studioSection="Maskieren"
                    Task { await model.generateMask(region) }
                } label: {
                    VStack(spacing:8) {
                        Image(systemName:region.symbol).font(.system(size:22,weight:.medium)).frame(width:60,height:54)
                            .background(.black.opacity(0.2),in:RoundedRectangle(cornerRadius:9))
                            .overlay(RoundedRectangle(cornerRadius:9).stroke(.white.opacity(0.2)))
                        Text(region.rawValue).font(.caption)
                    }.foregroundStyle(.white.opacity(0.75))
                }.buttonStyle(.plain).disabled(locked).accessibilityIdentifier("auto-"+region.rawValue)
            }
        } }
    }
    private var dock: some View {
        VStack(spacing:0) {
            if studioSection == "Bearbeiten" {
                ScrollView(.horizontal,showsIndicators:false) { HStack(spacing:10) {
                    ForEach(StudioTool.allCases.filter { ![.masks,.looks,.crop].contains($0) },id:\.self) { item in
                        Button { tool=item } label: {
                            VStack(spacing:4) {
                                Image(systemName:item.symbol).font(.system(size:18,weight:.medium)).frame(height:22)
                                Text(item.rawValue).font(.system(size:10))
                                Circle().fill(tool == item ? .white : .clear).frame(width:4,height:4)
                            }.frame(width:57,height:50).foregroundStyle(.white.opacity(tool == item ? 0.95 : 0.55))
                        }.disabled(locked).accessibilityIdentifier("tool-"+item.rawValue)
                    }
                }.padding(.horizontal,12) }.accessibilityIdentifier("tool-dock")
            }
            Rectangle().fill(.white.opacity(0.12)).frame(height:1)
            HStack(spacing:0) {
                sectionButton("Aktionen",symbol:"wand.and.stars",target:nil)
                sectionButton("Looks",symbol:"camera.filters",target:.looks)
                sectionButton("Retusche",symbol:"face.smiling.fill",target:nil)
                sectionButton("Zuschnitt",symbol:"crop.rotate",target:.crop)
                sectionButton("Bearbeiten",symbol:"slider.horizontal.3",target:.light)
                sectionButton("Maskieren",symbol:"circle.dashed.inset.filled",target:.masks)
            }.padding(.horizontal,6).padding(.top,6).padding(.bottom,4)
        }.background(Color(white:0.11).opacity(0.94))
    }
    private func sectionButton(_ name:String,symbol:String,target:StudioTool?) -> some View {
        Button {
            studioSection=name
            if let target { tool=target }
            if target == .crop { setCrop("Frei") }
        } label: {
            VStack(spacing:5) {
                Image(systemName:symbol).font(.system(size:18,weight:.medium)).frame(width:36,height:32)
                    .background(studioSection == name ? Color.blue : Color.clear,in:RoundedRectangle(cornerRadius:8))
                Text(name).font(.system(size:9)).lineLimit(1).minimumScaleFactor(0.8)
            }.frame(maxWidth:.infinity).frame(height:47).foregroundStyle(.white.opacity(studioSection == name ? 1 : 0.65))
        }.buttonStyle(.plain).disabled(locked).accessibilityIdentifier(target == .masks ? "tool-Masken" : "section-"+name)
    }
    @ViewBuilder private var toolControls: some View {
        switch tool {
        case .light:
            glass {
                VStack(spacing:6) {
                    control("Belichtung",key:"exposure",path:\.exposure,range:-4...4,step:0.05,suffix:" EV")
                    control("Kontrast",key:"contrast",path:\.contrast,range:-100...100)
                    control("Lichter",key:"highlights",path:\.highlights,range:-100...100)
                    control("Tiefen",key:"shadows",path:\.shadows,range:-100...100)
                    control("Weiß",key:"whites",path:\.whites,range:-100...100)
                    control("Schwarz",key:"blacks",path:\.blacks,range:-100...100)

                }
            }
        case .color:
            glass {
                VStack(spacing:10) {
                    tabs(["Farbe","Farbmix","Grading"],selection:$colorTab)
                    if colorTab == "Farbmix" {
                        HStack { ForEach(0..<8,id:\.self) { index in
                            Button { colorBand=index } label: {
                                Circle().fill([Color.red,.orange,.yellow,.green,.cyan,.blue,.purple,.pink][index]).frame(width:24,height:24).overlay(Circle().stroke(colorBand == index ? .white : .clear,lineWidth:2))
                            }.accessibilityLabel(["Rot","Orange","Gelb","Grün","Aqua","Blau","Violett","Magenta"][index])
                        } }
                        mixControl("Farbton",path:\.hue)
                        mixControl("Sättigung",path:\.saturation)
                        mixControl("Luminanz",path:\.luminance)
                    } else if colorTab == "Grading" {
                        Picker("Bereich",selection:$gradingZone) { Text("Tiefen").tag(0);Text("Mitte").tag(1);Text("Lichter").tag(2) }.pickerStyle(.segmented)
                        GradingWheel(grade:$model.advanced.grades[gradingZone],changed:advancedChanged,finished:advancedFinished)
                        advancedSlider("Farbton",value:$model.advanced.grades[gradingZone].hue,range:0...360)
                        advancedSlider("Intensität",value:$model.advanced.grades[gradingZone].saturation,range:0...100)
                        advancedSlider("Helligkeit",value:$model.advanced.grades[gradingZone].luminance,range:-100...100)
                        advancedSlider("Balance",value:$model.advanced.balance,range:-100...100)
                        control("Warme Lichter",key:"gradeWarm",path:\.gradeWarm,range:-100...100)
                        control("Kühle Tiefen",key:"gradeCool",path:\.gradeCool,range:-100...100)
                        control("Matte Schwarztöne",key:"fade",path:\.fade,range:0...100)
                    } else {
                        Button { samplingWhiteBalance.toggle() } label: { Label(samplingWhiteBalance ? "Pipette beenden" : (model.rawSettings == nil ? "Weißabgleich · Pipette" : "RGB-Pipette"),systemImage:"eyedropper") }
                        Button("Schwarzweiß") { model.development.saturation = model.development.saturation == -100 ? 0 : -100; model.previewChanges(); Task { await model.commitDevelopment() } }
                    if model.rawSettings != nil {
                        Text("RAW · Originalentwicklung").font(.caption).foregroundStyle(.blue)
                        Button("Weißabgleich wie aufgenommen") { Task { await model.resetRAW() } }
                        advancedSlider("Temperatur · Kelvin",value:rawBinding(\.temperature),range:2000...50000)
                        advancedSlider("RAW-Tönung",value:rawBinding(\.tint),range:-150...150)
                    }
                    control("Wärme",key:"warmth",path:\.warmth,range:-100...100)
                    control("Tönung",key:"tint",path:\.tint,range:-100...100)
                    control("Dynamik",key:"vibrance",path:\.vibrance,range:-100...100)
                    control("Sättigung",key:"saturation",path:\.saturation,range:-100...100)
                    }
                }
            }
        case .effects: effectsPanel
        case .details: detailsPanel
        case .masks: masksPanel
        case .curve: curvePanel
        case .optics:
            glass { VStack(spacing:10) {
                Text("Optik & Geometrie").font(.headline)
                if let raw=model.rawSettings {
                    if raw.lensSupported {
                        Toggle("RAW-Objektivkorrektur",isOn:Binding(get:{model.rawSettings?.lensCorrection ?? false},set:{model.rawSettings?.lensCorrection=$0;advancedChanged();advancedFinished()}))
                    } else { Text("Für dieses RAW bietet iOS keine Objektivkorrektur.").font(.caption) }
                }
                advancedSlider("Begradigen",value:$model.advanced.angle,range:-20...20)
                advancedSlider("Vertikale Perspektive",value:$model.advanced.vertical,range:-100...100)
                advancedSlider("Horizontale Perspektive",value:$model.advanced.horizontal,range:-100...100)
                control("Verzeichnung",key:"lens",path:\.lens,range:-100...100)
                control("Rote Farbsäume",key:"redFringe",path:\.redFringe,range:-100...100)
                control("Blaue Farbsäume",key:"blueFringe",path:\.blueFringe,range:-100...100)
            } }
        case .looks: looksPanel
        case .crop: cropPanel
        case .layers: layersPanel
        }
    }
    private func panelTitle(_ title:String,subtitle:String) -> some View {
        HStack {
            VStack(alignment:.leading,spacing:4) {
                Text(title).font(.system(size:12,weight:.semibold))
            }
            Spacer()
            Button { showReset=true } label: { Image(systemName:"arrow.counterclockwise").frame(width:36,height:36) }
                .accessibilityLabel("Anpassungen zurücksetzen").disabled(locked)
            Image(systemName:"square.on.square").frame(width:36,height:36)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance:0).onChanged { _ in
                    comparing=true; Task { await model.compare() }
                }.onEnded { _ in comparing=false })
                .accessibilityLabel("Ohne Anpassungen vergleichen")
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { comparing.toggle(); if comparing { Task { await model.compare() } } }
        }
    }
    @ViewBuilder private func control(_ title:String,key:String,path:WritableKeyPath<Development,Double>,range:ClosedRange<Double>,step:Double=1,suffix:String="") -> some View {
        if true {
            VStack(spacing:0) {
                HStack {
                    Text(title).font(.system(size:12,weight:.regular))
                    Spacer()
                    Text(model.development[keyPath:path].formatted(.number.precision(.fractionLength(step < 1 ? 2 : 0))) + suffix)
                        .font(.system(size:12,weight:.medium,design:.monospaced)).foregroundStyle(.white.opacity(0.8))
                }
                StudioSlider(value:Binding(get:{model.development[keyPath:path]},set:{value in
                    model.development[keyPath:path]=value; model.previewChanges()
                }),range:range,identifier:"slider-"+key,label:title,onEditing:{editing in
                    dragging=editing ? key : nil
                    if !editing { Task { await model.commitDevelopment() } }
                }).frame(height:28)
                    .disabled(locked)
            }.opacity(dragging == nil || dragging == key ? 1 : 0.12)
        }
    }
    private func tabs(_ items:[String],selection:Binding<String>) -> some View {
        HStack { ForEach(items,id:\.self) { label in
            Button(label) { selection.wrappedValue=label }.font(.subheadline)
                .foregroundStyle(selection.wrappedValue == label ? .white : .gray)
                .padding(.vertical,5).frame(maxWidth:.infinity)
                .overlay(alignment:.bottom) { if selection.wrappedValue == label { Rectangle().fill(.white).frame(height:2) } }
        } }
    }
    private func mixControl(_ label:String,path:WritableKeyPath<ColorBand,Double>) -> some View {
        VStack {
            HStack { Text(label); Spacer(); Text(model.development.mix[colorBand][keyPath:path].formatted(.number.precision(.fractionLength(0)))).monospacedDigit() }
            StudioSlider(value:Binding(get:{model.development.mix[colorBand][keyPath:path]},set:{model.development.mix[colorBand][keyPath:path]=$0;model.previewChanges()}),range:-100...100,identifier:"mix-"+label,label:label,onEditing:{ if !$0 { Task { await model.commitDevelopment() } } }).frame(height:30)
        }
    }
    private var effectsPanel: some View {
        glass { VStack(spacing:10) {
            tabs(["Effekte","Vignette","Körnung"],selection:$effectTab)
            if effectTab == "Effekte" {
                control("Struktur",key:"texture",path:\.texture,range:-100...100)
                control("Klarheit",key:"clarity",path:\.clarity,range:-100...100)
                control("Dunst entfernen",key:"dehaze",path:\.dehaze,range:-100...100)
                control("Unschärfe",key:"blur",path:\.blur,range:0...100)
            } else if effectTab == "Vignette" {
                control("Vignette",key:"vignette",path:\.vignette,range:-100...100)
                control("Mittelpunkt",key:"vignetteMid",path:\.vignetteMid,range:0...100)
            } else { control("Körnung",key:"grain",path:\.grain,range:0...100) }
        } }
    }
    private var detailsPanel: some View {
        glass { VStack(spacing:10) {
            tabs(["Schärfen","Rauschen","Farbrauschen"],selection:$detailTab)
            if detailTab == "Schärfen" {
                control("Schärfen",key:"sharpness",path:\.sharpness,range:0...150,step:1)
                control("Radius",key:"sharpRadius",path:\.sharpRadius,range:0...3,step:0.1)
                control("Details",key:"sharpDetail",path:\.sharpDetail,range:0...100,step:1)
                control("Maskieren",key:"sharpMask",path:\.sharpMask,range:0...100,step:1)
            } else if detailTab == "Rauschen" {
                control("Luminanz",key:"noise",path:\.noise,range:0...100)
                control("Details",key:"noiseDetail",path:\.noiseDetail,range:0...100)
                control("Kontrast",key:"noiseContrast",path:\.noiseContrast,range:0...100)
            } else {
                control("Farbrauschreduzierung",key:"colorNoise",path:\.colorNoise,range:0...100)
                control("Details",key:"colorDetail",path:\.colorDetail,range:0...100)
                control("Glättung",key:"colorSmooth",path:\.colorSmooth,range:0...100)
            }
        } }
    }
    private var curvePanel: some View {
        glass { VStack(spacing:10) {
            Text("Tonkurve").font(.headline)
            Picker("Kanal",selection:$curveChannel) { Text("RGB").tag(0);Text("Rot").tag(1);Text("Grün").tag(2);Text("Blau").tag(3) }.pickerStyle(.segmented)
            CurveEditor(points:$model.advanced.curves[curveChannel],changed:advancedChanged,finished:advancedFinished)
            Button("Kanal zurücksetzen") { model.advanced.curves[curveChannel]=AdvancedSettings().curves[curveChannel];advancedChanged();advancedFinished() }
            control("Tiefen",key:"curveShadows",path:\.curveShadows,range:-100...100)
            control("Mitteltöne",key:"curveMid",path:\.curveMid,range:-100...100)
            control("Lichter",key:"curveHighlights",path:\.curveHighlights,range:-100...100)
        } }
    }
    private var masksPanel: some View {
        glass { VStack(alignment:.leading,spacing:8) {
            if model.advanced.angle != 0 || model.advanced.vertical != 0 || model.advanced.horizontal != 0 {
                Text("Geometriekorrektur während der Maskenbearbeitung ausgeblendet.").font(.caption)
            }
            HStack {
                Menu {
                    Section("Automatisch") { ForEach(AutomaticRegion.allCases) { region in Button(region.rawValue,systemImage:region.symbol) { Task { await model.generateMask(region) } } } }
                    Section("Manuell") { Button("Pinsel") { model.addMask("brush") }; Button("Linearer Verlauf") { model.addMask("linear") }; Button("Radialer Verlauf") { model.addMask("radial") } } } label: { Label("Neue Maske",systemImage:"plus") }.accessibilityIdentifier("new-mask")
                Spacer()
                if model.selectedMask != nil { Button(role:.destructive) { model.removeMask() } label: { Image(systemName:"trash") }.accessibilityLabel("Maske löschen") }
                Button { maskOverlay.toggle() } label: { Image(systemName:maskOverlay ? "eye" : "eye.slash") }.accessibilityLabel("Maskenüberlagerung").accessibilityIdentifier("mask-overlay").accessibilityValue(maskOverlay ? "Sichtbar" : "Ausgeblendet")
            }
            ScrollView(.horizontal) { HStack { ForEach(model.masks) { mask in
                Button(mask.name) { model.selectedMask=mask.id }.padding(8).background(model.selectedMask == mask.id ? Color.blue : Color.white.opacity(0.1),in:Capsule())
            } } }
            DisclosureGroup("Farb- und Helligkeitsbereich") {
                Toggle("Farbbereich",isOn:$rangeColor)
                HStack { Text(rangeColor ? "Farbton" : "Helligkeit");Slider(value:$rangeCenter,in:0...1) }
                HStack { Text("Toleranz");Slider(value:$rangeTolerance,in:0.01...0.5) }
                Button("Bereich auswählen") { Task { await model.rangeMask(color:rangeColor,center:rangeCenter,tolerance:rangeTolerance) } }
            }
            if let mask=model.masks.first(where:{$0.id == model.selectedMask}) {
                Text(mask.kind == "ai" ? "Auswahl prüfen. Mit dem Pinsel ergänzen oder radieren." : mask.kind == "brush" ? "Mit dem Finger auf dem Bild malen." : "Auf dem Bild ziehen, um die Maske zu setzen.").font(.caption).foregroundStyle(.secondary)
                if mask.kind == "brush" || mask.kind == "ai" {
                    if UIDevice.current.userInterfaceIdiom == .pad {
                        Toggle("Nur Apple Pencil",isOn:$pencilOnly).accessibilityIdentifier("pencil-only")
                    }
                    Toggle("Radieren",isOn:$erasing)
                    HStack { Text("Pinselgröße"); StudioSlider(value:$brushRadius,range:0...0.2,identifier:"brush-size",label:"Pinselgröße",onEditing:{_ in}).frame(height:30) }
                }
                maskControl("Weiche Kante",mask:mask,path:\.feather,range:0...1)
                maskControl("Belichtung",mask:mask,path:\.exposure,range:-4...4)
                maskControl("Wärme",mask:mask,path:\.warmth,range:-100...100)
                maskControl("Sättigung",mask:mask,path:\.saturation,range:-100...100)
                ForEach(localSettings,id:\.0) { label,path in
                    advancedSlider(label,value:localBinding(mask,path:path),range:-100...100)
                }
                Toggle("Umkehren",isOn:maskToggle(mask,path:\.inverted))
                Toggle("Aktiv",isOn:maskToggle(mask,path:\.enabled))

            } else {
                automaticChoices([.subject,.background,.sky,.objects,.skin,.hair,.eyes])
                Text("Oder unter Neue Maske einen Pinsel oder Verlauf wählen.").font(.caption).foregroundStyle(.secondary)
            }
        }.font(.subheadline) }.disabled(locked)
    }
    // SwiftUI may finish a gesture or read a binding after its view was removed.
    // Resolve by stable ID on every access; never retain an array position.
    private func maskBinding(_ snapshot:LocalMask) -> Binding<LocalMask> {
        Binding(get:{ model.masks.first(where:{$0.id == snapshot.id}) ?? snapshot },set:{ updated in
            guard let index=model.masks.firstIndex(where:{$0.id == snapshot.id}) else { return }
            model.masks[index]=updated
        })
    }
    private func maskToggle(_ mask:LocalMask,path:WritableKeyPath<LocalMask,Bool>) -> Binding<Bool> {
        let binding=maskBinding(mask)
        return Binding(get:{binding.wrappedValue[keyPath:path]},set:{value in
            var updated=binding.wrappedValue; updated[keyPath:path]=value; binding.wrappedValue=updated
            model.previewChanges(); Task { await model.commitDevelopment() }
        })
    }
    private func maskControl(_ label:String,mask:LocalMask,path:WritableKeyPath<LocalMask,Double>,range:ClosedRange<Double>) -> some View {
        let binding=maskBinding(mask)
        return VStack {
            HStack { Text(label); Spacer(); Text(binding.wrappedValue[keyPath:path].formatted(.number.precision(.fractionLength(2)))).monospacedDigit() }
            StudioSlider(value:Binding(get:{binding.wrappedValue[keyPath:path]},set:{value in
                maskOverlay=false;var updated=binding.wrappedValue; updated[keyPath:path]=value; binding.wrappedValue=updated;model.previewChanges()
            }),range:range,identifier:"mask-"+label,label:label,onEditing:{ maskOverlay=false; if !$0 { Task { await model.commitDevelopment() } } }).frame(height:30)
        }.id(mask.id+label)
    }
    private var looksPanel: some View {
        glass {
            VStack(alignment:.leading,spacing:16) {
            savedLooksPanel
                Text("Ein Gefühl. Ein Look.").font(.subheadline.weight(.medium))
                ScrollView(.horizontal,showsIndicators:false) {
                    HStack(spacing:10) {
                        look("Natürlich",colors:[.gray,.white.opacity(0.3)],settings:Development())
                        look("Golden",colors:[.orange.opacity(0.7),.brown],settings:Development(exposure:0.15,contrast:12,saturation:-5,vibrance:18,warmth:25))
                        look("Nordic",colors:[.cyan.opacity(0.45),.gray],settings:Development(exposure:0.3,contrast:-12,saturation:-20,vibrance:5,warmth:-18))
                        look("Noir",colors:[.gray,.black],settings:Development(exposure:0.1,contrast:35,saturation:-100))
                        look("Lebendig",colors:[.green.opacity(0.6),.orange.opacity(0.65)],settings:Development(contrast:15,saturation:12,vibrance:30))
                    }
                }
                Text("Cinema").font(.subheadline.weight(.semibold))
                LazyVGrid(columns:[GridItem(.flexible()),GridItem(.flexible())],spacing:16) {
                    ForEach(CinemaLook.allCases,id:\.self) { preset in
                        Button { Task { await model.applyPreset(preset.settings) } } label: {
                            VStack(alignment:.leading,spacing:6) {
                                lookThumbnail(preset.rawValue,settings:preset.settings).frame(height:88).clipped().contentShape(Rectangle())
                                Text(preset.rawValue).font(.caption.weight(.medium))
                                Text(preset.subtitle).font(.system(size:10)).foregroundStyle(.secondary)
                            }.contentShape(Rectangle())
                        }.buttonStyle(.plain).disabled(locked).accessibilityIdentifier("cinema-"+preset.rawValue)
                    }
                }
            }
        }
    }
    private func lookThumbnail(_ name:String,settings:Development) -> some View {
        Group {
            if let image=model.lookThumbnails[name] {
                Image(uiImage:image).resizable().scaledToFill()
                    .accessibilityLabel("Fotovorschau "+name).accessibilityIdentifier("look-preview-"+name)
            } else { Rectangle().fill(.white.opacity(0.06)).overlay(ProgressView().tint(.white)) }
        }.task(id:model.lookPreviewRevision) { await model.loadLookThumbnail(name,settings:settings) }
    }
    private func look(_ title:String,colors:[Color],settings:Development) -> some View {
        Button { Task { await model.applyPreset(settings) } } label: {
            VStack(spacing:8) {
                lookThumbnail(title,settings:settings).frame(width:88,height:72).clipped().clipShape(RoundedRectangle(cornerRadius:8)).contentShape(Rectangle())
                Text(title).font(.system(size:10))
            }
        }.disabled(locked)
    }
    private var cropPanel:some View {
        glass { VStack(spacing:8) {
            HStack {
                Button("Auswahl verwerfen",systemImage:"xmark") {setCrop("Frei");studioSection="Bearbeiten";tool = .light}.labelStyle(.iconOnly).accessibilityLabel("Auswahl verwerfen")
                Spacer();Text(cropName).font(.caption.weight(.semibold));Spacer()
                Button("Anwenden",systemImage:"checkmark") {if cropTab == "Geometrie" {studioSection="Bearbeiten";tool = .light}else{applyCrop()}}.labelStyle(.iconOnly).accessibilityLabel("Anwenden").disabled(locked)
            }.frame(height:28)
            Picker("Zuschnitt-Werkzeuge",selection:$cropTab) {Text("Format").tag("Format");Text("Geometrie").tag("Geometrie")}.pickerStyle(.segmented)
            if cropTab == "Format" {
                ScrollView(.horizontal,showsIndicators:false) {HStack(spacing:14) {
                    Button("Original",systemImage:"photo") {setCrop("Original")}
                    Menu {Button("Frei") {setCrop("Frei")};ForEach(CropGeometry.ratios,id:\.0) {item in Button(item.0) {setCrop(item.0,ratio:item.1)}}} label:{Label("Verhältnisse",systemImage:"aspectratio")}.accessibilityIdentifier("crop-ratios")
                    Menu {Button("Quadrat · 1:1") {setCrop("Instagram · 1:1",ratio:1)};Button("Hochformat · 4:5") {setCrop("Instagram · 4:5",ratio:0.8)};Button("Story / Reel · 9:16") {setCrop("Instagram · 9:16",ratio:9.0/16)}} label:{Text("Instagram")}
                    Button("TikTok · 9:16") {setCrop("TikTok · 9:16",ratio:9.0/16)}
                    Button("KI-Erweitern",systemImage:"arrow.up.left.and.arrow.down.right") {aiExpansion=true}
                }.buttonStyle(.borderless).font(.caption).frame(height:32)}.accessibilityIdentifier("crop-formats")
                HStack(spacing:18) {
                    Button {if cropRatio != nil {cropRatio=nil;cropName="Frei"}else if let d=model.document {cropRatio=cropRect.width*Double(d.width)/(cropRect.height*Double(d.height));cropName="Benutzerdefiniert"}} label:{Image(systemName:cropRatio == nil ? "lock.open":"lock.fill")}.accessibilityLabel("Seitenverhältnis sperren").accessibilityValue(cropRatio == nil ? "Frei":"Gesperrt")
                    Button {if let ratio=cropRatio {setCrop(cropName.hasSuffix(" · gedreht") ? cropName.replacingOccurrences(of:" · gedreht",with:"") : cropName+" · gedreht",ratio:1/ratio)}} label:{Image(systemName:"rectangle.portrait.rotate")}.disabled(cropRatio == nil).accessibilityLabel("Hochformat / Querformat")
                    Spacer()
                    Button {Task {await model.command("image.imageRotation.90cw");setCrop("Frei")}} label:{Image(systemName:"rotate.right")}.accessibilityLabel("90 Grad drehen")
                    Button {Task {await model.command("image.imageRotation.flipCanvasHorizontal")}} label:{Image(systemName:"arrow.left.and.right.righttriangle.left.righttriangle.right")}.accessibilityLabel("Horizontal spiegeln")
                }.frame(height:30).disabled(locked)
            } else {
                Text("Geometrie wird direkt gespeichert. Rückgängig ist oben verfügbar.").font(.caption2).foregroundStyle(.secondary)
                advancedSlider("Begradigen",value:$model.advanced.angle,range:-20...20)
                advancedSlider("Vertikale Perspektive",value:$model.advanced.vertical,range:-100...100)
                advancedSlider("Horizontale Perspektive",value:$model.advanced.horizontal,range:-100...100)
            }
        } }
    }
    private func setCrop(_ label:String,ratio:Double?=nil) {
        guard let doc=model.document else {return}
        cropName=label
        cropRatio=label == "Original" ? model.originalAspectRatio:ratio
        if let cropRatio {cropRect=CropGeometry.fit(ratio:cropRatio,image:CGSize(width:doc.width,height:doc.height))}
        else {cropRect=CGRect(x:0,y:0,width:1,height:1)}
    }
    private func applyCrop() {
        guard let d=model.document else { return }
        let x=max(0,Int(cropRect.minX*Double(d.width))), y=max(0,Int(cropRect.minY*Double(d.height)))
        let w=max(1,min(d.width-x,Int(cropRect.width*Double(d.width))))
        let h=max(1,min(d.height-y,Int(cropRect.height*Double(d.height))))
        Task { await model.command("image.crop",["x":x,"y":y,"width":w,"height":h]); setCrop("Frei");studioSection="Bearbeiten";tool = .light }
    }
    private var layersPanel: some View {
        glass {
            VStack(spacing:10) {
                HStack {
                    Text("Ebenen").font(.subheadline.weight(.medium)); Spacer()
                    Text("\(model.document?.layers.count ?? 0) / 24").font(.caption).foregroundStyle(.white.opacity(0.5))
                    Menu {
                        Button("Duplizieren") { Task { await model.command("layer.duplicate") } }
                        Button("Löschen",role:.destructive) { Task { await model.command("layer.delete") } }
                    } label: { Image(systemName:"ellipsis.circle").frame(width:32,height:32) }.disabled(locked)
                }
                ScrollView {
                    VStack(spacing:3) {
                        ForEach(model.document?.layers ?? []) { layer in
                            HStack {
                                icon(layer.visible ? "Ausblenden" : "Einblenden",layer.visible ? "eye" : "eye.slash") {
                                    Task { await model.command("layer.setProps",["layer":layer.id,"visible":!layer.visible]) }
                                }
                                Button { Task { await model.command("layer.select",["layer":layer.id]) } } label: {
                                    HStack { Text(layer.name.replacingOccurrences(of:"PhotoCraft · ",with:"")); Spacer()
                                        if model.document?.activeLayer == layer.id { Image(systemName:"checkmark").font(.caption) }
                                    }.font(.caption).frame(height:44)
                                }
                            }.padding(.horizontal,5).background(.white.opacity(model.document?.activeLayer == layer.id ? 0.09 : 0),in:RoundedRectangle(cornerRadius:12))
                        }
                    }
                }.frame(maxHeight:180).disabled(locked)
            }
        }
    }
    private func presentPendingImport() {
        let source = pendingImport
        pendingImport = nil
        switch source {
        case .photos: pickingPhoto = true
        case .files: files = true
        case nil: break
        }
    }
    private func updateSpatialEditing() {
        model.spatialEditing=controls && (studioSection == "Maskieren" || (studioSection == "Zuschnitt" && cropTab == "Format") || repairMode)
        model.previewChanges()
    }
    private func advancedChanged() { model.previewChanges() }
    private func advancedFinished() { Task { await model.commitDevelopment() } }
    private func rawBinding(_ path:WritableKeyPath<RAWSettings,Double>)->Binding<Double> {
        Binding(get:{model.rawSettings?[keyPath:path] ?? 0},set:{model.rawSettings?[keyPath:path]=$0})
    }
    private func advancedSlider(_ title:String,value:Binding<Double>,range:ClosedRange<Double>)->some View {
        VStack {
            HStack { Text(title);Spacer();Text(value.wrappedValue.formatted(.number.precision(.fractionLength(0)))).monospacedDigit() }
            StudioSlider(value:Binding(get:{value.wrappedValue},set:{value.wrappedValue=$0;advancedChanged()}),range:range,identifier:"advanced-"+title,label:title,onEditing:{if studioSection == "Maskieren" {maskOverlay=false};if !$0 {advancedFinished()}}).frame(height:32)
        }.font(.caption)
    }
    private var localSettings:[(String,WritableKeyPath<Development,Double>)] { [("Kontrast",\.contrast),("Lichter",\.highlights),("Tiefen",\.shadows),("Weiß",\.whites),("Schwarz",\.blacks),("Tönung",\.tint),("Struktur",\.texture),("Klarheit",\.clarity),("Dunst entfernen",\.dehaze)] }
    private func localBinding(_ mask:LocalMask,path:WritableKeyPath<Development,Double>)->Binding<Double> {
        let binding=maskBinding(mask)
        return Binding(get:{binding.wrappedValue.adjustments?[keyPath:path] ?? 0},set:{value in
            maskOverlay=false;var updated=binding.wrappedValue;var settings=updated.adjustments ?? Development();settings[keyPath:path]=value;updated.adjustments=settings;binding.wrappedValue=updated
        })
    }
    private var savedLooksPanel:some View {
        VStack(alignment:.leading,spacing:8) {
            Button {referenceLook=true} label:{Label("Look aus Referenzfoto",systemImage:"photo.on.rectangle.angled")}.accessibilityIdentifier("reference-look")
            Text("Eigene Presets").font(.headline)
            Button("Preset importieren (XMP / PicsRoom)") { importingPreset=true;files=true }
            Text("Lightroom: Licht- und Farbwerte werden angenähert. Profile, Masken und weitere Adobe-Effekte werden nicht übernommen.").font(.caption).foregroundStyle(.secondary)
            HStack { TextField("Name",text:$presetName);Button("Speichern") {model.saveLook(presetName);presetName=""}.disabled(presetName.trimmingCharacters(in:.whitespaces).isEmpty) }
            ForEach(model.catalog.presets) { preset in
                HStack { Button(preset.name) { Task { await model.applyLook(preset) } };Spacer();Button { model.exportLook(preset) } label:{Image(systemName:"square.and.arrow.up")};Button(role:.destructive){model.deleteLook(preset.id)}label:{Image(systemName:"trash")} }
            }
        }
    }
    private var projectLibrary: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing:20) {
                    HStack {
                        Button { pendingImport = .photos; library=false } label: { Label("Foto öffnen",systemImage:"plus") }.buttonStyle(.borderedProminent).accessibilityIdentifier("library-import-photo")
                        Button { importingPreset=false; pendingImport = .files; library=false } label: { Label("Dateien",systemImage:"folder") }.buttonStyle(.bordered).accessibilityIdentifier("library-import-file")
                    }.padding(.top)
                    HStack {
                        TextField("Albumname",text:$albumName).textFieldStyle(.roundedBorder)
                        Button("Zuordnen") { model.organize(selectedProjects,album:albumName) }.disabled(selectedProjects.isEmpty)
                    }
                    Picker("Album",selection:$albumFilter) {
                        Text("Alle").tag("Alle")
                        ForEach(Array(Set(model.catalog.entries.values.map(\.album))).filter{!$0.isEmpty}.sorted(),id:\.self) { Text($0).tag($0) }
                    }
                    Picker("Bewertung",selection:$minimumRating) { ForEach(0...5,id:\.self) { Text($0==0 ? "Alle Bewertungen" : "Ab \($0) ★").tag($0) } }
                    if !selectedProjects.isEmpty {
                        Button("Aktuelle Bearbeitung auf \(selectedProjects.count) Fotos übertragen") { Task { await model.batchApply(selectedProjects) } }.disabled(model.batchRunning)
                        Text("Übernimmt Licht, Farbe, Kurven und Grading. Masken, RAW-Weißabgleich und Zuschnitt bleiben individuell.").font(.caption)
                    }
                    if model.projects.isEmpty {
                        ContentUnavailableView("Dein Studio wartet",systemImage:"photo.stack",description:Text("Öffne ein Foto. Deine Projekte werden hier automatisch gespeichert."))
                    }
                    LazyVGrid(columns:[GridItem(.adaptive(minimum:145))],spacing:16) {
                        ForEach(model.projects.filter { project in
                            let entry=model.catalog.entries[project.url.lastPathComponent] ?? CatalogEntry()
                            return entry.rating>=minimumRating && (albumFilter == "Alle" || entry.album==albumFilter)
                        }) { project in
                            VStack {
                            Toggle("Auswählen",isOn:Binding(get:{selectedProjects.contains(project.url)},set:{ if $0 {selectedProjects.insert(project.url)}else{selectedProjects.remove(project.url)} })).font(.caption)
                            Button { selectedProjects=[];library=false; Task { await model.openSelection([project.url]) } } label: {
                                VStack(alignment:.leading,spacing:8) {
                                    Group {
                                        if let image=project.thumbnail { Image(uiImage:image).resizable().scaledToFill() }
                                        else { Rectangle().fill(.white.opacity(0.08)).overlay(Image(systemName:"photo")) }
                                    }.frame(height:180).clipped().clipShape(RoundedRectangle(cornerRadius:18)).contentShape(Rectangle())
                                    Text(project.date.formatted(date:.abbreviated,time:.shortened)).font(.caption).foregroundStyle(.secondary)
                                }.contentShape(Rectangle())
                            }.buttonStyle(.plain)
                            HStack(spacing:4) {
                                ForEach(1...5,id:\.self) { rating in
                                    Button { let current=model.catalog.entries[project.url.lastPathComponent]?.rating ?? 0;model.organize([project.url],rating:current==rating ? 0 : rating) } label: { Image(systemName:(model.catalog.entries[project.url.lastPathComponent]?.rating ?? 0)>=rating ? "star.fill":"star").font(.caption).foregroundStyle(.yellow) }
                                }
                            }
                            }
                        }
                    }
                }.padding(.horizontal,16).disabled(model.batchRunning)
            }.navigationTitle("Dein Studio").toolbar { ToolbarItem(placement:.confirmationAction) { Button("Fertig") { let urls=model.projects.map(\.url).filter{selectedProjects.contains($0)};library=false;selectedProjects=[];if !urls.isEmpty {Task {await model.openSelection(urls)}} } } }
        }.preferredColorScheme(.dark)
    }
}
struct ShareSheet: UIViewControllerRepresentable {
    let url: URL
    func makeUIViewController(context: Context) -> UIActivityViewController {
        let controller=UIActivityViewController(activityItems:[url],applicationActivities:nil)
        controller.modalPresentationStyle = .pageSheet
        return controller
    }
    func updateUIViewController(_ controller: UIActivityViewController,context: Context) {}
}

private struct StudioPanelHeightKey: EnvironmentKey { static let defaultValue:CGFloat=230 }
extension EnvironmentValues {
    var studioPanelHeight:CGFloat {
        get { self[StudioPanelHeightKey.self] }
        set { self[StudioPanelHeightKey.self]=newValue }
    }
}
private struct StudioPanel<Content:View>:View {
    @Environment(\.studioPanelHeight) private var height
    let opaque:Bool
    let content:Content
    var body:some View {
        ScrollView { content.font(.system(size:12)).padding(.horizontal,16).padding(.vertical,8) }
            .frame(height:height).scrollIndicators(.visible).accessibilityIdentifier("settings-panel")
            .background(.black.opacity(opaque ? 0.96 : 0.56))
    }
}
