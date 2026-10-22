import SwiftUI
import Photos

struct ExportOptions {
    var format="jpg"
    var maxSide=0
    var quality=0.95
    var colorSpace="sRGB"
    var watermark=""
    var filename="PicsRoom"
}

struct ExportView:View {
    @ObservedObject var model:EditorModel
    @Environment(\.dismiss) private var dismiss
    @State private var selected:Set<URL>=[]
    @State private var options=ExportOptions()
    @State private var size=0
    @State private var customSize=3000
    @State private var watermark=false
    @State private var watermarkText=""
    @State private var running=false
    @State private var result:[URL]=[]
    @State private var share=false
    @State private var message:String?
    private var candidates:[SavedProject] {
        let urls=model.editingProjects
        return urls.count>1 ? urls.compactMap { url in model.projects.first{$0.url==url} } : model.projects
    }
    private var rendered:Bool { !["original","pcraft"].contains(options.format) }
    var body:some View {
        NavigationStack {
            Form {
                Section("Fotos auswählen · \(selected.count)") {
                    ScrollView(.horizontal) {
                        LazyHStack(spacing:12) {
                            ForEach(candidates) { project in
                                Button { if selected.contains(project.url) {selected.remove(project.url)}else{selected.insert(project.url)} } label: {
                                    VStack(spacing:6) {
                                        Group { if let image=project.thumbnail {Image(uiImage:image).resizable().scaledToFill()}else{Color.gray} }
                                            .frame(width:86,height:105).clipped().clipShape(RoundedRectangle(cornerRadius:8))
                                            .overlay(RoundedRectangle(cornerRadius:8).stroke(selected.contains(project.url) ? Color.blue : Color.clear,lineWidth:3))
                                        Image(systemName:selected.contains(project.url) ? "checkmark.square.fill":"square")
                                    }
                                }.buttonStyle(.plain).accessibilityLabel("Exportfoto \(project.url.lastPathComponent)").accessibilityValue(selected.contains(project.url) ? "Ausgewählt":"Nicht ausgewählt")
                            }
                        }.padding(4)
                    }
                    Button(selected.count==candidates.count ? "Auswahl aufheben":"Alle auswählen") {selected=selected.count==candidates.count ? [] : Set(candidates.map(\.url))}
                }
                Section("Exportieren als …") {
                    Picker("Dateityp",selection:$options.format) {
                        Text("JPG").tag("jpg");Text("PNG").tag("png");Text("TIFF · 16 Bit").tag("tiff");Text("HEIF · HDR 10 Bit").tag("hdr")
                        Text("Unverändertes Original").tag("original");Text("PicsRoom-Projekt").tag("pcraft")
                    }.accessibilityIdentifier("export-format")
                    if rendered {
                        Picker("Größe",selection:$size) {
                            Text("Größte verfügbare Abmessungen").tag(0);Text("2048 px · lange Kante").tag(2048);Text("4096 px · lange Kante").tag(4096);Text("Benutzerdefiniert").tag(-1)
                        }.accessibilityIdentifier("export-size")
                        if size == -1 { HStack {Text("Lange Kante · Pixel");TextField("Pixel",value:$customSize,format:.number.grouping(.never)).keyboardType(.numberPad).multilineTextAlignment(.trailing)} }
                        Text("Seitenverhältnis bleibt erhalten. Kleine Bilder werden nicht hochskaliert.").font(.caption).foregroundStyle(.secondary)
                        if options.format == "jpg" {
                            Picker("Bildqualität",selection:$options.quality) {ForEach([0.6,0.75,0.85,0.95,1.0],id:\.self) {Text("\(Int($0*100)) %").tag($0)}}.accessibilityIdentifier("export-quality")
                        }
                        Toggle("Wasserzeichen einschließen",isOn:$watermark)
                        if watermark {TextField("Dein Wasserzeichen",text:$watermarkText).onChange(of:watermarkText) {_,text in watermarkText=String(text.prefix(80))}}
                    } else {Text("Original und Projekt werden unverändert ausgegeben. Größe, Qualität und Wasserzeichen werden nicht angewendet.").font(.caption).foregroundStyle(.secondary)}
                }
                Section("Weitere Optionen") {
                    TextField("Dateiname",text:$options.filename)
                    if rendered && options.format != "hdr" {
                        Picker("Farbraum",selection:$options.colorSpace) {Text("sRGB").tag("sRGB");Text("Display P3").tag("P3")}
                    }
                    Text(options.format == "hdr" ? "HDR wird als HLG / Rec.2020 ausgegeben. Die Vorschau im Editor ist SDR." : "Gerenderte Exporte enthalten keine übernommenen Standort- oder Kamera-Metadaten. Originaldateien behalten ihre Metadaten.").font(.caption).foregroundStyle(.secondary)
                }
                Section {
                    Button {start(savePhotos:false)} label:{Label("Teilen / In Dateien sichern",systemImage:"square.and.arrow.up")}.accessibilityIdentifier("export-share")
                    Button {start(savePhotos:true)} label:{Label("In Fotos speichern",systemImage:"square.and.arrow.down")}.disabled(!rendered).accessibilityIdentifier("export-photos")
                    if running {HStack {ProgressView();Text(model.status).font(.caption)}}
                }
            }.disabled(running)
            .navigationTitle("Exportieren (\(selected.count))").navigationBarTitleDisplayMode(.inline)
            .toolbar {ToolbarItem(placement:.cancellationAction) {Button("Schließen",systemImage:"xmark") {dismiss()}.disabled(running)}}
            .interactiveDismissDisabled(running)
            .sheet(isPresented:$share) {ExportShareSheet(urls:result)}
            .alert("PicsRoom",isPresented:Binding(get:{message != nil},set:{if !$0 {message=nil}})) {Button("OK"){message=nil}} message:{Text(message ?? "")}
            .onAppear {model.refreshProjects();if let url=model.projectURL {selected=[url]}}
        }.preferredColorScheme(.dark).tint(.blue)
    }
    private func start(savePhotos:Bool) {
        guard !running,!selected.isEmpty else {message="Bitte mindestens ein Foto auswählen.";return}
        if watermark && rendered && watermarkText.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty {message="Bitte einen Text für das Wasserzeichen eingeben.";return}
        if rendered && size == -1 && !(256...12000).contains(customSize) {message="Bitte eine lange Kante zwischen 256 und 12000 Pixeln eingeben.";return}
        var settings=options;settings.maxSide=size == -1 ? customSize : size;settings.watermark=watermark ? watermarkText : ""
        let urls=candidates.map(\.url).filter{selected.contains($0)}
        running=true
        Task {
            defer {running=false}
            do {
                if savePhotos {
                    let permission=await PHPhotoLibrary.requestAuthorization(for:.addOnly)
                    guard permission == .authorized || permission == .limited else {throw EditorError(message:"Bitte in den iOS-Einstellungen den Zugriff zum Hinzufügen von Fotos erlauben.")}
                }
                result=try await model.exportSelection(urls,options:settings)
                if savePhotos {
                    let files=result
                    try await PHPhotoLibrary.shared().performChanges {
                        for url in files {PHAssetCreationRequest.forAsset().addResource(with:.photo,fileURL:url,options:nil)}
                    }
                    message="\(files.count) Foto(s) in Fotos gespeichert."
                } else {share=true}
            } catch {message=error.localizedDescription;model.error=nil}
        }
    }
}
private struct ExportShareSheet:UIViewControllerRepresentable {
    let urls:[URL]
    func makeUIViewController(context:Context)->UIActivityViewController {UIActivityViewController(activityItems:urls,applicationActivities:nil)}
    func updateUIViewController(_ controller:UIActivityViewController,context:Context) {}
}
