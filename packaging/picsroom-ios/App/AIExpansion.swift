import SwiftUI
import Security

/// Personal-device credential only. Never included in a project, archive, log or UserDefaults.
enum ExpansionKey {
    private static let query:[String:Any]=[kSecClass as String:kSecClassGenericPassword,kSecAttrService as String:"PicsRoom.OpenAI",kSecAttrAccount as String:"image-edit"]
    static func read()->String? {
        var q=query;q[kSecReturnData as String]=true;q[kSecMatchLimit as String]=kSecMatchLimitOne
        var item:CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary,&item)==errSecSuccess,let data=item as? Data else {return nil}
        return String(data:data,encoding:.utf8)
    }
    static func save(_ key:String) throws {
        let clean=key.trimmingCharacters(in:.whitespacesAndNewlines)
        guard !clean.isEmpty,!clean.contains("\n"),!clean.contains("\r") else {throw EditorError(message:"Bitte einen gültigen API-Schlüssel eingeben.")}
        let attrs:[String:Any]=[kSecValueData as String:Data(clean.utf8),kSecAttrAccessible as String:kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        var status=SecItemUpdate(query as CFDictionary,attrs as CFDictionary)
        if status==errSecItemNotFound {status=SecItemAdd(query.merging(attrs){_,new in new} as CFDictionary,nil)}
        guard status==errSecSuccess else {throw EditorError(message:"Der Schlüssel konnte nicht im Schlüsselbund gespeichert werden.")}
    }
    static func delete() {SecItemDelete(query as CFDictionary)}
}

struct ExpansionCanvas {
    let source:UIImage
    let size:CGSize
    let placement:CGRect
    let image:Data
    let mask:Data
    static func prepare(_ image:UIImage,ratio:Double,border:Double) throws -> Self {
        guard image.size.width>0,image.size.height>0,(1.0/3...3).contains(ratio),(0...0.4).contains(border) else {throw EditorError(message:"Bitte ein Seitenverhältnis zwischen 1:3 und 3:1 wählen.")}
        let width=ratio>=1 ? 1536.0 : (1536*ratio/16).rounded()*16
        let height=ratio>=1 ? (1536/ratio/16).rounded()*16 : 1536.0
        let size=CGSize(width:width,height:height)
        let scale=min(width/image.size.width,height/image.size.height)*(1-border)
        let w=floor(image.size.width*scale),h=floor(image.size.height*scale)
        let rect=CGRect(x:floor((width-w)/2),y:floor((height-h)/2),width:w,height:h)
        let format=UIGraphicsImageRendererFormat();format.scale=1;format.preferredRange = .standard
        let renderer=UIGraphicsImageRenderer(size:size,format:format)
        let canvas=renderer.image {_ in image.draw(in:rect)}
        let mask=renderer.image {context in UIColor.white.setFill();context.fill(rect)}
        guard let input=canvas.pngData(),let matte=mask.pngData() else {throw EditorError(message:"Die KI-Vorlage konnte nicht vorbereitet werden.")}
        return Self(source:image,size:size,placement:rect,image:input,mask:matte)
    }
    func merge(_ generated:UIImage) throws -> UIImage {
        guard generated.size==size else {throw EditorError(message:"Der Bilddienst hat eine unerwartete Bildgröße geliefert. Bitte erneut versuchen.")}
        let format=UIGraphicsImageRendererFormat();format.scale=1;format.preferredRange = .standard
        return UIGraphicsImageRenderer(size:size,format:format).image {_ in
            generated.draw(in:CGRect(origin:.zero,size:size))
            // Preserve the existing photo at the output resolution, irrespective of model changes inside the mask.
            source.draw(in:placement)
        }
    }
}

private final class ExpansionSessionDelegate:NSObject,URLSessionTaskDelegate {
    func urlSession(_ session:URLSession,task:URLSessionTask,willPerformHTTPRedirection response:HTTPURLResponse,newRequest request:URLRequest,completionHandler:@escaping(URLRequest?)->Void) {completionHandler(nil)}
}
enum ExpansionService {
    static func request(canvas:ExpansionCanvas,key:String,instruction:String) throws -> URLRequest {
        var request=URLRequest(url:URL(string:"https://api.openai.com/v1/images/edits")!)
        request.httpMethod="POST";request.timeoutInterval=240
        request.setValue("Bearer "+key,forHTTPHeaderField:"Authorization");request.setValue("application/json",forHTTPHeaderField:"Content-Type")
        let prompt="Extend this photograph into the transparent area of the canvas. Fill only the transparent masked region with a natural continuation of the scene. Match lighting, perspective, depth of field and grain. Keep the existing photograph and all people unchanged, in exactly the same location and scale. Do not add text, frames or watermarks. Additional direction: "+String(instruction.prefix(1000))
        request.httpBody=try JSONSerialization.data(withJSONObject:["model":"gpt-image-2","images":[["image_url":"data:image/png;base64,"+canvas.image.base64EncodedString()]],"mask":["image_url":"data:image/png;base64,"+canvas.mask.base64EncodedString()],"prompt":prompt,"size":"\(Int(canvas.size.width))x\(Int(canvas.size.height))","quality":"medium","output_format":"png","n":1])
        return request
    }
    static func generate(canvas:ExpansionCanvas,key:String,instruction:String,session:URLSession?=nil) async throws -> UIImage {
        let config=URLSessionConfiguration.ephemeral;config.timeoutIntervalForRequest=240;config.timeoutIntervalForResource=300;config.httpShouldSetCookies=false;config.urlCache=nil
        let client=session ?? URLSession(configuration:config,delegate:ExpansionSessionDelegate(),delegateQueue:nil)
        defer {if session==nil {client.invalidateAndCancel()}}
        let (data,response)=try await client.data(for:request(canvas:canvas,key:key,instruction:instruction))
        try Task.checkCancellation()
        guard let response=response as? HTTPURLResponse else {throw EditorError(message:"Keine gültige Antwort vom Bilddienst.")}
        guard response.statusCode==200 else {
            let message:String
            switch response.statusCode {
            case 401:message="Der API-Schlüssel wurde nicht akzeptiert. Bitte den KI-Zugang prüfen."
            case 403:message="Der API-Zugang hat keinen Zugriff auf das Bildmodell. Bitte Freischaltung und Projektberechtigungen prüfen."
            case 429:message="API-Limit oder Guthaben erreicht. Bitte den API-Zugang prüfen und später erneut versuchen."
            default:message="Die KI-Erweiterung ist fehlgeschlagen (HTTP \(response.statusCode)). Dein bisheriges Bild bleibt erhalten."
            };throw EditorError(message:message)
        }
        guard data.count<40_000_000,let object=try JSONSerialization.jsonObject(with:data) as? [String:Any],let rows=object["data"] as? [[String:Any]],let base64=rows.first?["b64_json"] as? String,let bytes=Data(base64Encoded:base64),let image=UIImage(data:bytes) else {throw EditorError(message:"Der Bilddienst hat kein lesbares Bild zurückgegeben.")}
        return try canvas.merge(image)
    }
}

struct AIExpansionView:View {
    @ObservedObject var model:EditorModel
    let initialRatio:Double?
    @Environment(\.dismiss) private var dismiss
    @State private var source:UIImage?
    @State private var canvas:ExpansionCanvas?
    @State private var output:UIImage?
    @State private var ratio=1.0
    @State private var border=0.2
    @State private var instruction=""
    @State private var key=""
    @State private var hasKey=false
    @State private var preparing=true
    @State private var working=false
    @State private var consent=false
    @State private var message:String?
    @State private var operation:Task<Void,Never>?
    var body:some View {
        NavigationStack {
            Form {
                Section {
                    if let image=output ?? canvas.flatMap({UIImage(data:$0.image)}) {
                        Image(uiImage:image).resizable().scaledToFit().frame(maxHeight:250).frame(maxWidth:.infinity).background(Color.gray.opacity(0.25))
                    } else {ProgressView("Bild vorbereiten …")}
                    if let canvas {Text("Neue Bildgröße: \(Int(canvas.size.width)) × \(Int(canvas.size.height)) Pixel").font(.caption)}
                }
                if output==nil {
                    Section("Erweiterung") {
                        Picker("Zielformat",selection:$ratio) {ForEach(CropGeometry.ratios,id:\.0) {Text($0.0).tag($0.1)};Text("16:9").tag(16.0/9)}
                        HStack {Text("Zusätzlicher Rand");Slider(value:$border,in:0.05...0.4,onEditingChanged:{editing in if !editing {prepare()}})}
                        TextField("Optional: Was soll ergänzt werden?",text:$instruction,axis:.vertical).lineLimit(2...4)
                    }.disabled(working || preparing)
                    Section("KI-Zugang · OpenAI") {
                        if hasKey {Label("Schlüssel sicher auf diesem Gerät gespeichert",systemImage:"key.fill");Button("Zugang entfernen",role:.destructive) {ExpansionKey.delete();hasKey=false}}
                        else {
                            SecureField("Persönlicher API-Schlüssel",text:$key).textInputAutocapitalization(.never).autocorrectionDisabled()
                            Button("Im Schlüsselbund speichern") {do {try ExpansionKey.save(key);key="";hasKey=true}catch{message=error.localizedDescription}}
                            Link("API-Schlüssel verwalten",destination:URL(string:"https://platform.openai.com/api-keys")!)
                        }
                    }.disabled(working)
                    Section {
                        Text("Bei Start werden dieses Foto und die Beschreibung an OpenAI übertragen. Die API-Anfrage kann Kosten verursachen. Es wird eine neue Bilddatei mit bis zu 1536 Pixeln langer Kante erstellt; dein bisheriges Projekt bleibt erhalten.").font(.caption)
                        Toggle("Foto für diese Erweiterung an OpenAI senden",isOn:$consent).disabled(working)
                        Button("Mit KI erweitern") {generate()}.disabled(!hasKey || !consent || canvas==nil || working || preparing).accessibilityIdentifier("ai-expand-start")
                        if working {ProgressView("Bild wird erweitert …");Button("Anfrage abbrechen",role:.cancel) {operation?.cancel()}}
                    }
                } else {
                    Section {
                        Text("Ergebnis prüfen. Der vorhandene Bildbereich wurde in der Ausgabeauflösung erhalten; der zusätzliche Rand wurde generiert.").font(.caption)
                        Button("Als neues Bild übernehmen") {
                            guard let data=output?.pngData() else {return};working=true
                            operation=Task {let previous=model.projectURL;await model.importPhoto(data);working=false;if model.projectURL != previous {model.editingProjects=[];dismiss()}else {message=model.error ?? "Das neue Bild konnte nicht gespeichert werden.";model.error=nil}}
                        }.disabled(working).accessibilityIdentifier("ai-expand-accept")
                        Button("Ergebnis verwerfen") {output=nil;consent=false}.disabled(working)
                    }
                }
            }.navigationTitle("KI-Erweitern").navigationBarTitleDisplayMode(.inline)
                .toolbar {ToolbarItem(placement:.cancellationAction) {Button("Schließen") {operation?.cancel();dismiss()}.disabled(working && output != nil)}}
                .interactiveDismissDisabled(working)
                .onChange(of:ratio) {_,_ in prepare()}
                .onDisappear {operation?.cancel()}
                .alert("PicsRoom",isPresented:Binding(get:{message != nil},set:{if !$0 {message=nil}})) {Button("OK"){message=nil}} message:{Text(message ?? "")}
                .task {
                    hasKey=ExpansionKey.read() != nil
                    if let initialRatio, (CropGeometry.ratios.map(\.1)+[16.0/9]).contains(initialRatio) {ratio=initialRatio}
                    var options=ExportOptions();options.maxSide=1536
                    if let url=await model.export("png",options:options,present:false) {source=UIImage(contentsOfFile:url.path);try? FileManager.default.removeItem(at:url)}
                    preparing=false;prepare()
                    if source==nil {message=model.error ?? "Das Bild konnte nicht vorbereitet werden.";model.error=nil}
                }
        }.preferredColorScheme(.dark).tint(.blue)
    }
    private func prepare() {guard let source else {return};do {canvas=try ExpansionCanvas.prepare(source,ratio:ratio,border:border)}catch {canvas=nil;message=error.localizedDescription}}
    private func generate() {
        guard let canvas,let key=ExpansionKey.read(),consent,!working else {return}
        working=true
        operation=Task {
            defer {working=false}
            do {output=try await ExpansionService.generate(canvas:canvas,key:key,instruction:instruction)}
            catch {message=Task.isCancelled ? "Anfrage abgebrochen. Falls die Verarbeitung beim Anbieter bereits begonnen hat, können dennoch Kosten entstehen.":error.localizedDescription}
        }
    }
}

#if DEBUG
/// Offline transport fixture. This never calls the network and never reads a real API credential.
private final class ExpansionMockProtocol:URLProtocol {
    static var code=200
    static var responseData=Data()
    static var validRequest=false
    override class func canInit(with request:URLRequest)->Bool {true}
    override class func canonicalRequest(for request:URLRequest)->URLRequest {request}
    override func startLoading() {
        Self.validRequest=request.url?.absoluteString=="https://api.openai.com/v1/images/edits" && request.value(forHTTPHeaderField:"Authorization")=="Bearer test-only"
        let response=HTTPURLResponse(url:request.url!,statusCode:Self.code,httpVersion:nil,headerFields:["Content-Type":"application/json"])!
        client?.urlProtocol(self,didReceive:response,cacheStoragePolicy:.notAllowed);client?.urlProtocol(self,didLoad:Self.responseData);client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
enum ExpansionChecks {
    static func run() async throws {
        func require(_ condition:Bool,_ message:String) throws {if !condition {throw EditorError(message:message)}}
        for image in [CGSize(width:1200,height:1600),CGSize(width:6000,height:4000)] {
            for (_,ratio) in CropGeometry.ratios {
                let rect=CropGeometry.fit(ratio:ratio,image:image)
                for handle in 0..<8 {
                    for delta in [CGSize(width:0.2,height:0.1),CGSize(width:-0.3,height:-0.4),CGSize(width:2,height:2)] {
                        let r=CropGeometry.resize(rect,handle:handle,delta:delta,ratio:ratio*image.height/image.width)
                        try require(r.width>0 && r.height>0 && r.minX > -0.00001 && r.maxX<1.00001 && r.minY > -0.00001 && r.maxY<1.00001,"Zuschnitt verlässt das Bild")
                        try require(abs(r.width*image.width/(r.height*image.height)-ratio)<0.00001,"Formatsperre verloren")
                    }
                }
            }
        }
        let format=UIGraphicsImageRendererFormat();format.scale=1
        let source=UIGraphicsImageRenderer(size:CGSize(width:120,height:160),format:format).image {context in UIColor.red.setFill();context.fill(CGRect(x:0,y:0,width:120,height:160))}
        let canvas=try ExpansionCanvas.prepare(source,ratio:16.0/9,border:0.2)
        func pixel(_ image:UIImage,_ x:Int,_ y:Int)->[UInt8] {
            var bytes=[UInt8](repeating:0,count:4)
            let ci=CIImage(cgImage:image.cgImage!)
            DevelopRenderer.shared.context.render(ci,toBitmap:&bytes,rowBytes:4,bounds:CGRect(x:x,y:y,width:1,height:1),format:.RGBA8,colorSpace:CGColorSpace(name:CGColorSpace.sRGB)!)
            return bytes
        }
        let mask=UIImage(data:canvas.mask)!
        try require(pixel(mask,0,0)[3]==0 && pixel(mask,Int(canvas.size.width/2),Int(canvas.size.height/2))[3]==255,"KI-Maske ist nicht korrekt")
        let request=try ExpansionService.request(canvas:canvas,key:"test-only",instruction:"Natur")
        let json=try JSONSerialization.jsonObject(with:request.httpBody!) as! [String:Any]
        try require(json["model"] as? String=="gpt-image-2" && json["size"] as? String=="1536x864" && json["mask"] != nil,"KI-Anfrage unvollständig")
        let generated=UIGraphicsImageRenderer(size:canvas.size,format:format).image {context in UIColor.blue.setFill();context.fill(CGRect(origin:.zero,size:canvas.size))}
        ExpansionMockProtocol.responseData=try JSONSerialization.data(withJSONObject:["data":[["b64_json":generated.pngData()!.base64EncodedString()]]])
        let config=URLSessionConfiguration.ephemeral;config.protocolClasses=[ExpansionMockProtocol.self]
        let session=URLSession(configuration:config);defer {session.invalidateAndCancel()}
        let result=try await ExpansionService.generate(canvas:canvas,key:"test-only",instruction:"Natur",session:session)
        try require(ExpansionMockProtocol.validRequest,"Falsches API-Ziel")
        try require(pixel(result,0,0)[2]>240 && pixel(result,Int(canvas.size.width/2),Int(canvas.size.height/2))[0]>240,"Vorhandenes Motiv wurde nicht erhalten")
        ExpansionMockProtocol.code=401
        do {_=try await ExpansionService.generate(canvas:canvas,key:"test-only",instruction:"",session:session);throw EditorError(message:"401 nicht erkannt")}
        catch let error as EditorError {try require(error.message.contains("API-Schlüssel"),"Fehlerbehandlung fehlt")}
        ExpansionMockProtocol.code=200;ExpansionMockProtocol.responseData=Data("{}".utf8)
        do {_=try await ExpansionService.generate(canvas:canvas,key:"test-only",instruction:"",session:session);throw EditorError(message:"Leere Antwort akzeptiert")}
        catch let error as EditorError {try require(error.message.contains("lesbares Bild"),"Ungültige Antwort nicht erkannt")}
    }
}
#endif
