import SwiftUI

@main struct PhotoCraftApp: App {
    @StateObject private var editor = EditorModel()
    var body: some Scene {
        WindowGroup {
            EditorView(model: editor)
                .preferredColorScheme(.dark)
                .tint(Color(red:0.68,green:0.83,blue:0.88))
                .task { await editor.restore() }
        }
    }
}
