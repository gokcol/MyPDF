// The only bridge that receives document text creates an installed-model session.
// The download window receives language identifiers only. No URLs, networking,
// document logging, or on-disk translation cache exist in this implementation.
import AppKit
import Foundation
import SwiftUI
import Translation

private typealias Callback = @convention(c) (UInt64, UnsafePointer<CChar>) -> Void

private struct Request: Decodable {
    let operation: String
    let source: String?
    let target: String?
    let texts: [String]?
}

@available(macOS 26.0, *)
@MainActor
private final class Work {
    let id: UInt64
    let callback: Callback
    var task: Task<Void, Never>?
    var session: TranslationSession?
    var window: NSWindow?
    var windowDelegate: DownloadWindowDelegate?

    init(id: UInt64, callback: @escaping Callback) {
        self.id = id
        self.callback = callback
    }
    func emit(_ event: String, value: Any? = nil) {
        var message: [String: Any] = ["event": event]
        if let value { message["value"] = value }
        guard let data = try? JSONSerialization.data(withJSONObject: message),
              let json = String(data: data, encoding: .utf8) else { return }
        json.withCString { callback(id, $0) }
    }
}

@available(macOS 26.0, *)
@MainActor
private final class Registry {
    static let shared = Registry()
    var work: [UInt64: Work] = [:]

    func finish(_ id: UInt64, event: String = "finished", value: Any? = nil) {
        guard let item = work.removeValue(forKey: id) else { return }
        item.emit(event, value: value)
        item.window?.close()
        item.window = nil
        item.windowDelegate = nil
        item.session = nil
        item.task = nil
    }

    func cancel(_ id: UInt64) {
        guard let item = work[id] else { return }
        item.task?.cancel()
        item.session?.cancel()
        finish(id, event: "cancelled")
    }

    func start(id: UInt64, request: Request, callback: @escaping Callback) {
        let item = Work(id: id, callback: callback)
        work[id] = item
        item.task = Task {
            if request.operation == "languages" {
                let languages = await LanguageAvailability().supportedLanguages
                guard !Task.isCancelled, work[id] != nil else { return }
                let locale = Locale(identifier: "en")
                let rows = languages.map { language in
                    let code = language.minimalIdentifier
                    return ["id": code, "name": locale.localizedString(forIdentifier: code) ?? code]
                }.sorted { $0["name", default: ""] < $1["name", default: ""] }
                item.emit("languages", value: rows)
                finish(id)
                return
            }
            guard let sourceID = request.source, let targetID = request.target else {
                finish(id, event: "failed", value: "Choose a source and target language.")
                return
            }
            let source = Locale.Language(identifier: sourceID)
            let target = Locale.Language(identifier: targetID)
            if request.operation == "translate" {
                guard let texts = request.texts else {
                    finish(id, event: "failed", value: "There is no text to translate.")
                    return
                }
                // Create the cancellable installed-only session before any
                // service await. A catalog preflight could stall while retaining
                // document text, and this initializer already forbids downloads.
                await translate(item, source: source, target: target, texts: texts)
                return
            }
            let status = await LanguageAvailability().status(from: source, to: target)
            guard !Task.isCancelled, work[id] != nil else { return }
            if request.operation == "check" {
                let value: String
                switch status {
                case .installed: value = "installed"
                case .supported: value = "download_required"
                case .unsupported: value = "unsupported"
                @unknown default: value = "unsupported"
                }
                item.emit("availability", value: value)
                finish(id)
                return
            }
            guard status != .unsupported else {
                finish(id, event: "failed", value: "Apple does not support this language pair on this Mac. No cloud fallback is used.")
                return
            }
            if request.operation == "download" {
                // Model preparation must never receive PDF content, even if an
                // invalid future caller accidentally adds it to the JSON.
                guard request.texts == nil else {
                    finish(id, event: "failed", value: "Language preparation must not contain document text.")
                    return
                }
                if status == .installed {
                    item.emit("availability", value: "installed")
                    finish(id)
                } else {
                    showDownload(item, source: source, target: target)
                }
                return
            }
            finish(id, event: "failed", value: "Unknown local translation request.")
        }
    }

    private func translate(_ item: Work, source: Locale.Language, target: Locale.Language, texts: [String]) async {
        let id = item.id
        // This constructor is explicitly restricted to installed models.
        // A runtime assertion protects the privacy boundary if Apple changes
        // its behavior in a future OS. Never use the SwiftUI download session.
        let session = TranslationSession(installedSource: source, target: target)
        guard !session.canRequestDownloads else {
            finish(id, event: "failed", value: "Apple did not provide an offline-only session. Translation was stopped.")
            return
        }
        item.session = session
        item.emit("progress", value: ["completed": 0, "total": texts.count])
        do {
            for (index, text) in texts.enumerated() {
                try Task.checkCancellation()
                var translated: [String] = []
                for part in chunks(text) {
                    try Task.checkCancellation()
                    if part.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        translated.append(part)
                    } else {
                        let response = try await session.translate(part)
                        try Task.checkCancellation()
                        translated.append(response.targetText)
                    }
                }
                guard work[id] != nil else { return }
                item.emit("translated", value: ["index": index, "text": translated.joined(separator: "\n")])
                item.emit("progress", value: ["completed": index + 1, "total": texts.count])
            }
            finish(id)
        } catch {
            fail(id, error: error)
        }
    }

    // Bound a single inference while preserving paragraph boundaries where
    // possible. The page response keeps its caller-supplied index.
    private func chunks(_ text: String) -> [String] {
        var remaining = text[...]
        var parts: [String] = []
        while let limit = remaining.index(remaining.startIndex, offsetBy: 4_000, limitedBy: remaining.endIndex), limit != remaining.endIndex {
            let prefix = remaining[..<limit]
            let minimum = remaining.index(remaining.startIndex, offsetBy: 2_000)
            let boundary = prefix.lastIndex(where: { $0 == "\n" || $0 == " " })
            let end = boundary.flatMap { $0 >= minimum ? remaining.index(after: $0) : nil } ?? limit
            parts.append(String(remaining[..<end]))
            remaining = remaining[end...]
        }
        if !remaining.isEmpty { parts.append(String(remaining)) }
        return parts
    }

    func fail(_ id: UInt64, error: Error) {
        let message: String
        switch error {
        case is CancellationError, TranslationError.alreadyCancelled:
            finish(id, event: "cancelled")
            return
        case TranslationError.notInstalled:
            message = "These language models are not installed. Add the languages and try again."
        case TranslationError.unsupportedSourceLanguage, TranslationError.unsupportedTargetLanguage, TranslationError.unsupportedLanguagePairing:
            message = "Apple does not support this language pair on this Mac."
        case TranslationError.nothingToTranslate:
            message = "There is no text to translate. Scanned pages need local OCR first."
        default:
            // Never forward error descriptions: an OS error could include source
            // text or model internals. The UI receives a fixed, safe message.
            message = "Apple could not complete offline translation. Check the language downloads and try again."
        }
        finish(id, event: "failed", value: message)
    }

    private func showDownload(_ item: Work, source: Locale.Language, target: Locale.Language) {
        let id = item.id
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 450, height: 230),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Add Offline Languages"
        window.isReleasedWhenClosed = false
        let delegate = DownloadWindowDelegate { Registry.shared.cancel(id) }
        window.delegate = delegate
        window.contentView = NSHostingView(rootView: DownloadView(id: id, source: source, target: target))
        item.window = window
        item.windowDelegate = delegate
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
}

@available(macOS 26.0, *)
@MainActor
private final class DownloadWindowDelegate: NSObject, NSWindowDelegate {
    let onClose: () -> Void
    init(onClose: @escaping () -> Void) { self.onClose = onClose }
    func windowWillClose(_ notification: Notification) { onClose() }
}

@available(macOS 26.0, *)
private struct DownloadView: View {
    let id: UInt64
    let source: Locale.Language
    let target: Locale.Language
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Download languages for offline translation", systemImage: "arrow.down.circle")
                .font(.headline)
            Text("Apple may ask you to confirm a language download. No PDF text is included in this request. After downloading, return to MyPDF and choose Translate.")
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                ProgressView().controlSize(.small)
                Text("Preparing language models…").foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { Registry.shared.cancel(id) }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(width: 450, height: 230)
        .translationTask(source: source, target: target) { session in
            guard let item = Registry.shared.work[id] else { return }
            item.session = session
            do {
                // This session is for model preparation only. Never translate
                // document content through a session allowed to download.
                try await session.prepareTranslation()
                try Task.checkCancellation()
                guard Registry.shared.work[id] != nil else { return }
                item.emit("availability", value: "installed")
                Registry.shared.finish(id)
            } catch {
                Registry.shared.fail(id, error: error)
            }
        }
    }
}

@_cdecl("mypdf_translation_available")
public func translationAvailable() -> Bool {
    if #available(macOS 26.0, *) { return true }
    return false
}

@_cdecl("mypdf_translation_start")
public func translationStart(_ id: UInt64, _ json: UnsafePointer<CChar>, _ callback: @escaping @convention(c) (UInt64, UnsafePointer<CChar>) -> Void) {
    let data = Data(String(cString: json).utf8)
    let request = try? JSONDecoder().decode(Request.self, from: data)
    DispatchQueue.main.async {
        if #available(macOS 26.0, *), let request {
            Registry.shared.start(id: id, request: request, callback: callback)
        } else {
            let error = "{\"event\":\"failed\",\"value\":\"Offline Apple translation requires macOS 26 or later.\"}"
            error.withCString { callback(id, $0) }
        }
    }
}

@_cdecl("mypdf_translation_cancel")
public func translationCancel(_ id: UInt64) {
    DispatchQueue.main.async {
        if #available(macOS 26.0, *) { Registry.shared.cancel(id) }
    }
}
