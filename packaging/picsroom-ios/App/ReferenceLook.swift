import SwiftUI
import PhotosUI
import ImageIO
import UniformTypeIdentifiers

/// Statistical matching is intentionally conservative: scene content is not an editing recipe.
enum ReferenceMatch {
    struct Statistics {
        var median:Double; var spread:Double; var saturation:Double; var warm:Double; var tint:Double
    }
    static func decode(_ data:Data) throws -> UIImage {
        guard data.count <= 32*1024*1024,
              let source=CGImageSourceCreateWithData(data as CFData,nil),
              let cg=CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,kCGImageSourceCreateThumbnailWithTransform:true,kCGImageSourceThumbnailMaxPixelSize:1200] as CFDictionary)
        else { throw EditorError(message:"Kein lesbares Referenzfoto (maximal 32 MB). Bitte JPEG, PNG oder HEIC verwenden.") }
        return UIImage(cgImage:cg)
    }
    static func statistics(_ image:UIImage) throws -> Statistics {
        guard let cg=image.cgImage else {throw EditorError(message:"Das Foto konnte nicht analysiert werden.")}
        let n=64;var pixels=[UInt8](repeating:0,count:n*n*4)
        let drawn=pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let ctx=CGContext(data:bytes.baseAddress,width:n,height:n,bitsPerComponent:8,bytesPerRow:n*4,space:CGColorSpace(name:CGColorSpace.sRGB)!,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else{return false}
            ctx.draw(cg,in:CGRect(x:0,y:0,width:n,height:n));return true
        }
        guard drawn else {throw EditorError(message:"Bildanalyse fehlgeschlagen.")}
        var luminance=[Double](),sat=0.0,warm=0.0,tint=0.0
        for i in stride(from:0,to:pixels.count,by:4) where pixels[i+3]>240 {
            let r=Double(pixels[i])/255,g=Double(pixels[i+1])/255,b=Double(pixels[i+2])/255
            let l=0.2126*r+0.7152*g+0.0722*b
            luminance.append(l);sat += max(r,g,b)-min(r,g,b);warm += r-b;tint += (r+b)/2-g
        }
        guard luminance.count>16 else{throw EditorError(message:"Das Referenzfoto enthält zu wenig sichtbaren Bildinhalt.")}
        luminance.sort();let count=Double(luminance.count)
        return Statistics(median:luminance[luminance.count/2],spread:luminance[luminance.count*9/10]-luminance[luminance.count/10],saturation:sat/count,warm:warm/count,tint:tint/count)
    }
    static func suggested(base:Development,source:Statistics,reference:Statistics,amount:Double)->Development {
        func clamp(_ v:Double,_ limit:Double)->Double{max(-limit,min(limit,v))}
        let a=max(0,min(1,amount));if a==0{return base};var d=base
        d.exposure=clamp(base.exposure+a*clamp(log2(max(0.04,reference.median)/max(0.04,source.median))*1.3,1.5),5)
        d.contrast=clamp(base.contrast+a*clamp((reference.spread-source.spread)*100,40),100)
        d.saturation=clamp(base.saturation+a*clamp((reference.saturation-source.saturation)*180,45),100)
        d.warmth=clamp(base.warmth+a*clamp((reference.warm-source.warm)*220,65),100)
        d.tint=clamp(base.tint+a*clamp((reference.tint-source.tint)*220,60),100)
        return d
    }
    static func download(_ text:String) async throws -> Data {
        guard let url=URL(string:text.trimmingCharacters(in:.whitespacesAndNewlines)),url.scheme?.lowercased()=="https",url.host != nil,url.user==nil,url.password==nil else{throw EditorError(message:"Bitte einen vollständigen HTTPS-Bildlink eingeben.")}
        let configuration=URLSessionConfiguration.ephemeral;configuration.timeoutIntervalForRequest=30
        let session=URLSession(configuration:configuration);defer{session.invalidateAndCancel()}
        let (bytes,response)=try await session.bytes(from:url)
        guard let http=response as? HTTPURLResponse,(200...299).contains(http.statusCode),response.url?.scheme=="https" else{throw EditorError(message:"Der Bildlink ist nicht erreichbar.")}
        guard response.mimeType?.hasPrefix("image/")==true else{throw EditorError(message:"Der Link führt nicht direkt zu einem Foto. Bitte das Bild herunterladen und über Fotos oder Dateien auswählen.")}
        guard response.expectedContentLength<=32*1024*1024 else{throw EditorError(message:"Das Referenzfoto ist größer als 32 MB.")}
        var data=Data()
        for try await byte in bytes {data.append(byte);if data.count>32*1024*1024{throw EditorError(message:"Das Referenzfoto ist größer als 32 MB.")}}
        try Task.checkCancellation();return data
    }
}

struct ReferenceLookView:View {
    @ObservedObject var model:EditorModel
    @Environment(\.dismiss) private var dismiss
    @State private var photo:PhotosPickerItem?
    @State private var files=false
    @State private var link=""
    @State private var reference:UIImage?
    @State private var source:ReferenceMatch.Statistics?
    @State private var target:ReferenceMatch.Statistics?
    @State private var baseline=Development()
    @State private var candidate=Development()
    @State private var preview:UIImage?
    @State private var amount=0.75
    @State private var busy=true
    @State private var failure:String?
    @State private var job:Task<Void,Never>?
    @State private var revision=0
    var body:some View {
        NavigationStack {
            ScrollView {
                VStack(alignment:.leading,spacing:16) {
                    Text("Übernimm die Farb- und Lichtwirkung eines Referenzfotos. Die Analyse läuft auf deinem Gerät. Unterschiedliche Motive können das Ergebnis beeinflussen.").font(.subheadline).foregroundStyle(.secondary)
                    HStack {
                        PhotosPicker(selection:$photo,matching:.images,preferredItemEncoding:.current){Label("Fotos",systemImage:"photo")}
                        Button {files=true}label:{Label("Dateien",systemImage:"folder")}
                    }.buttonStyle(.bordered).disabled(busy)
                    HStack {
                        TextField("https://… direkter Bildlink",text:$link).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled().textFieldStyle(.roundedBorder)
                        Button("Laden"){load{try await ReferenceMatch.download(link)}}.disabled(busy || link.isEmpty)
                    }
                    Text("Bei Links wird nur das Referenzfoto geladen. Dein eigenes Foto wird nicht hochgeladen.").font(.caption).foregroundStyle(.secondary)
                    HStack(alignment:.top) {
                        photoView(reference,"Referenz")
                        photoView(preview ?? model.preview,"Dein Foto")
                    }
                    if busy {ProgressView("Foto wird analysiert …")}
                    if let failure {Text(failure).foregroundStyle(.red).accessibilityIdentifier("reference-error")}
                    if target != nil {
                        Text("Stärke: \(Int(amount*100)) %")
                        Slider(value:$amount,in:0...1,onEditingChanged:{editing in updatePreview()}).accessibilityIdentifier("reference-strength")
                        Text("Belichtung, Kontrast, Sättigung, Wärme und Tönung bleiben anschließend einzeln bearbeitbar. Du kannst das Ergebnis auch als eigenes Preset speichern.").font(.caption).foregroundStyle(.secondary)
                    }
                    #if DEBUG
                    if ProcessInfo.processInfo.arguments.contains("--test-reference") {
                        Button("Testreferenz") {load {let f=UIGraphicsImageRenderer(size:CGSize(width:128,height:128));return f.image{ctx in UIColor(red:0.7,green:0.4,blue:0.2,alpha:1).setFill();ctx.fill(CGRect(x:0,y:0,width:128,height:128))}.pngData()!}}.accessibilityIdentifier("reference-fixture")
                    }
                    #endif
                }.padding()
            }.navigationTitle("Referenz-Look").navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement:.cancellationAction){Button("Abbrechen"){dismiss()}}
                    ToolbarItem(placement:.confirmationAction){Button("Übernehmen") {job=Task {busy=true;await model.applyPreset(candidate);busy=false;if model.error==nil{dismiss()}else{failure=model.error}}}.disabled(busy || target==nil || source==nil).accessibilityIdentifier("reference-apply")}
                }
        }.preferredColorScheme(.dark)
            .task {baseline=model.development;candidate=baseline;do {let image=try await model.referencePreview(baseline);preview=image;source=try ReferenceMatch.statistics(image);busy=false}catch{failure=error.localizedDescription;busy=false}}
            .onDisappear{job?.cancel();revision += 1}
            .onChange(of:photo){_,value in guard let value else{return};load{guard let data=try await value.loadTransferable(type:Data.self) else{throw EditorError(message:"Das Foto konnte nicht geladen werden.")};return data}}
            .fileImporter(isPresented:$files,allowedContentTypes:[.image]) {result in
                switch result {case .success(let url):load {let granted=url.startAccessingSecurityScopedResource();defer{if granted{url.stopAccessingSecurityScopedResource()}};let size=try url.resourceValues(forKeys:[.fileSizeKey]).fileSize ?? 0;guard size<=32*1024*1024 else{throw EditorError(message:"Das Referenzfoto ist größer als 32 MB.")};return try Data(contentsOf:url)}
                case .failure(let error):failure=error.localizedDescription}
            }
    }
    private func photoView(_ image:UIImage?,_ title:String)->some View {
        VStack {Text(title).font(.caption);if let image{Image(uiImage:image).resizable().scaledToFit()}else{Image(systemName:"photo").font(.largeTitle).frame(height:160)}}.frame(maxWidth:.infinity)
    }
    private func load(_ operation:@escaping () async throws -> Data) {
        job?.cancel();busy=true;failure=nil;target=nil;revision += 1
        job=Task {do {let data=try await operation();try Task.checkCancellation();let image=try ReferenceMatch.decode(data);let stats=try ReferenceMatch.statistics(image)
                #if DEBUG
                if ProcessInfo.processInfo.arguments.contains("--test-reference"),let source {
                    assert(ReferenceMatch.suggested(base:baseline,source:source,reference:source,amount:1)==baseline)
                    assert(ReferenceMatch.suggested(base:baseline,source:source,reference:stats,amount:0)==baseline)
                    let full=ReferenceMatch.suggested(base:baseline,source:source,reference:stats,amount:1)
                    assert(full.sharpness==baseline.sharpness && full.mix==baseline.mix)
                    assert(full != baseline)
                    assert((try? ReferenceMatch.decode(Data("not an image".utf8))) == nil)
                }
                #endif
                reference=image;target=stats;busy=false;updatePreview()}catch{if !Task.isCancelled{busy=false;failure=error.localizedDescription}}}
    }
    private func updatePreview() {
        guard let source,let target else{return};revision += 1;let token=revision
        candidate=ReferenceMatch.suggested(base:baseline,source:source,reference:target,amount:amount)
        let values=candidate;busy=true
        job=Task {do {let image=try await model.referencePreview(values);guard token==revision,!Task.isCancelled else{return};preview=image;busy=false}catch{if token==revision{busy=false;failure=error.localizedDescription}}}
    }
}
