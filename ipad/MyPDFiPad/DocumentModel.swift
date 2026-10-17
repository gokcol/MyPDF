import Foundation
import PDFKit
import SwiftUI
import UniformTypeIdentifiers

struct LibraryItem: Codable, Identifiable {
  var id = UUID()
  var name: String
  var file: String
  var modified = Date()
}
struct PDFObject: Identifiable {
  var raw: [String: Any]
  var id: String { raw["id"] as? String ?? "" }
  var page: Int { raw["page"] as? Int ?? 0 }
  var rect: CGRect {
    let r = raw["rect"] as? [Double] ?? [0, 0, 1, 1]
    return CGRect(x: r[0], y: r[1], width: r[2] - r[0], height: r[3] - r[1])
  }
  var kind: [String: Any] { raw["kind"] as? [String: Any] ?? [:] }
  var isImage: Bool { kind["Image"] != nil }
  var canMove: Bool {
    (isImage || kind["Ink"] != nil) && abs(raw["rotation"] as? Double ?? 0) < 0.01
      && !(raw["flip_x"] as? Bool ?? false) && !(raw["flip_y"] as? Bool ?? false)
  }
  var label: String {
    isImage ? "Image / signature" : kind["Ink"] != nil ? "Drawing / signature" : "PDF object"
  }
  var style: [String: Any] { raw["style"] as? [String: Any] ?? [:] }
  var color: Color {
    let c = style["color"] as? [Double] ?? [0, 0, 0]
    return Color(red: c[0], green: c[1], blue: c[2])
  }
  var png: Data? {
    guard let a = kind["Image"] as? [String: Any], let b = a["png"] as? String else { return nil }
    return Data(base64Encoded: b)
  }
}

enum RustPDF {
  static func call(_ arguments: [String: Any]) throws -> [String: Any] {
    let input = try JSONSerialization.data(withJSONObject: arguments)
    guard let request = String(data: input, encoding: .utf8) else {
      throw failure("Invalid request")
    }
    return try request.withCString { pointer in
      guard let response = mypdf_ipad_call(pointer) else {
        throw failure("PDF engine returned no response")
      }
      defer { mypdf_ipad_free(response) }
      let data = Data(String(cString: response).utf8)
      let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
      guard parsed["ok"] as? Bool == true else {
        throw failure(parsed["error"] as? String ?? "PDF operation failed")
      }
      return parsed["result"] as? [String: Any] ?? [:]
    }
  }
  static func failure(_ text: String) -> NSError {
    NSError(domain: "MyPDF", code: 1, userInfo: [NSLocalizedDescriptionKey: text])
  }
  static func run(_ arguments: [String: Any]) async throws -> [String: Any] {
    try await withCheckedThrowingContinuation { continuation in
      DispatchQueue.global(qos: .userInitiated).async {
        continuation.resume(with: Result { try call(arguments) })
      }
    }
  }
}

@MainActor final class DocumentModel: ObservableObject {
  @Published var library: [LibraryItem] = []
  @Published var currentID: UUID?
  @Published var url: URL?
  @Published var document: PDFDocument?
  @Published var page = 0
  @Published var sizes: [CGSize] = []
  @Published var objects: [PDFObject] = []
  @Published var editable = false
  @Published var busy = false
  @Published var error: String?
  @Published var history: [URL] = []
  @Published var future: [URL] = []
  let root: URL
  var title: String { library.first { $0.id == currentID }?.name ?? "MyPDF" }
  var pageSize: CGSize {
    sizes.indices.contains(page) ? sizes[page] : CGSize(width: 595, height: 842)
  }
  var signatureURL: URL { root.appendingPathComponent("Signature.json") }
  init() {
    root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent("MyPDF", isDirectory: true)
    do {
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      if let data = try? Data(contentsOf: root.appendingPathComponent("Library.json")) {
        library = try JSONDecoder().decode([LibraryItem].self, from: data)
      }
    } catch { self.error = error.localizedDescription }
  }
  func fresh(_ ext: String = "pdf") -> URL {
    root.appendingPathComponent(UUID().uuidString).appendingPathExtension(ext)
  }
  /// Recover the retained drawings affected by the initial preview's row-button bug.
  /// Add separate library entries; never replace a current document.
  func recoverInitialDrawings() async {
    guard !UserDefaults.standard.bool(forKey: "recoveredInitialDrawingRevisions"), !busy else {
      return
    }
    busy = true
    defer { busy = false }
    do {
      let referenced = Set(library.map(\.file))
      let files = try FileManager.default.contentsOfDirectory(
        at: root, includingPropertiesForKeys: nil
      )
      .filter { $0.pathExtension == "pdf" && !referenced.contains($0.lastPathComponent) }.prefix(
        100)
      var count = 0
      for file in files {
        if let meta = try? await RustPDF.run(["command": "annotations", "input": file.path]),
          let marks = meta["annotations"] as? [[String: Any]], !marks.isEmpty
        {
          count += 1
          library.insert(
            LibraryItem(name: "Recovered drawing \(count)", file: file.lastPathComponent), at: 0)
        }
      }
      try persist()
      UserDefaults.standard.set(true, forKey: "recoveredInitialDrawingRevisions")
    } catch { self.error = error.localizedDescription }
  }
  func persist() throws {
    try JSONEncoder().encode(library).write(
      to: root.appendingPathComponent("Library.json"), options: .atomic)
  }
  func activate(_ item: LibraryItem) async {
    guard !busy else { return }
    busy = true
    defer { busy = false }
    do {
      try await load(root.appendingPathComponent(item.file))
      currentID = item.id
      history = []
      future = []
    } catch { self.error = error.localizedDescription }
  }
  func load(_ next: URL) async throws {
    let meta = try await RustPDF.run(["command": "annotations", "input": next.path])
    guard let pdf = PDFDocument(url: next), !pdf.isLocked else {
      throw RustPDF.failure(
        "Unlock password-protected PDFs on your desktop before using this test build.")
    }
    sizes = (meta["pages"] as? [[Double]] ?? []).map { CGSize(width: $0[0], height: $0[1]) }
    objects = (meta["annotations"] as? [[String: Any]] ?? []).map { PDFObject(raw: $0) }
    editable = meta["editable"] as? Bool ?? false
    document = pdf
    url = next
    page = min(page, max(0, pdf.pageCount - 1))
  }
  func newDocument() async {
    guard !busy else { return }
    busy = true
    defer { busy = false }
    do {
      let next = fresh()
      _ = try await RustPDF.run(["command": "blank", "output": next.path])
      try await addToLibrary(next, name: "Untitled")
    } catch { self.error = error.localizedDescription }
  }
  func importPDF(_ external: URL) async {
    guard !busy else { return }
    busy = true
    defer { busy = false }
    let scoped = external.startAccessingSecurityScopedResource()
    defer { if scoped { external.stopAccessingSecurityScopedResource() } }
    do {
      let size = try external.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
      guard size <= 256 * 1024 * 1024 else {
        throw RustPDF.failure("Open PDFs up to 256 MB in this test build.")
      }
      let next = fresh()
      try FileManager.default.copyItem(at: external, to: next)
      try await addToLibrary(next, name: external.deletingPathExtension().lastPathComponent)
    } catch { self.error = error.localizedDescription }
  }
  private func addToLibrary(_ next: URL, name: String) async throws {
    page = 0
    try await load(next)
    let item = LibraryItem(name: name, file: next.lastPathComponent)
    library.insert(item, at: 0)
    currentID = item.id
    history = []
    future = []
    try persist()
  }
  func mutate(_ parameters: [String: Any]) async {
    guard let previous = url, !busy, editable else { return }
    busy = true
    defer { busy = false }
    do {
      let next = fresh()
      var command = parameters
      command["input"] = previous.path
      command["output"] = next.path
      command["page"] = page
      _ = try await RustPDF.run(command)
      try await load(next)
      history.append(previous)
      if history.count > 30 { history.removeFirst() }
      future = []
      try updateLibrary(next)
    } catch { self.error = error.localizedDescription }
  }
  private func updateLibrary(_ next: URL) throws {
    if let index = library.firstIndex(where: { $0.id == currentID }) {
      library[index].file = next.lastPathComponent
      library[index].modified = Date()
      try persist()
    }
  }
  func undo(redo: Bool = false) async {
    guard !busy, let old = url, let next = (redo ? future.last : history.last) else { return }
    busy = true
    defer { busy = false }
    do {
      try await load(next)
      try updateLibrary(next)
      if redo {
        future.removeLast()
        history.append(old)
      } else {
        history.removeLast()
        future.append(old)
      }
    } catch { self.error = error.localizedDescription }
  }
  func extractPages(_ pages: [Int]) async {
    guard !busy, let source = url, !pages.isEmpty else { return }
    busy = true
    defer { busy = false }
    do {
      let output = fresh()
      let name = title + " — extracted pages"
      _ = try await RustPDF.run([
        "command": "pages", "action": "extract", "input": source.path, "output": output.path,
        "pages": pages,
      ])
      try await addToLibrary(output, name: name)
    } catch { self.error = error.localizedDescription }
  }
  func prepareObject(_ object: PDFObject) async throws -> UIImage {
    guard let input = url else { throw RustPDF.failure("Open a PDF") }
    let preview = fresh()
    defer { try? FileManager.default.removeItem(at: preview) }
    _ = try await RustPDF.run([
      "command": "preview_without", "input": input.path, "output": preview.path,
      "page": object.page, "id": object.id,
    ])
    guard let page = PDFDocument(url: preview)?.page(at: object.page) else {
      throw RustPDF.failure("Could not preview this page")
    }
    return page.thumbnail(of: CGSize(width: 1600, height: 2200), for: .cropBox)
  }
  func thumbnail() -> UIImage? {
    document?.page(at: page)?.thumbnail(of: CGSize(width: 1600, height: 2200), for: .cropBox)
  }
  func importImage(_ external: URL) throws -> URL {
    let scoped = external.startAccessingSecurityScopedResource()
    defer { if scoped { external.stopAccessingSecurityScopedResource() } }
    let info = try external.resourceValues(forKeys: [.fileSizeKey])
    guard (info.fileSize ?? 0) <= 32 * 1024 * 1024 else {
      throw RustPDF.failure("Choose an image smaller than 32 MB")
    }
    guard let image = UIImage(contentsOfFile: external.path), image.size.width > 0 else {
      throw RustPDF.failure("Choose a PNG or JPEG image")
    }
    let scale = min(1, 2048 / max(image.size.width, image.size.height))
    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let normalized = UIGraphicsImageRenderer(size: size, format: format).image { _ in
      image.draw(in: CGRect(origin: .zero, size: size))
    }
    guard let data = normalized.pngData(), data.count <= 8 * 1024 * 1024 else {
      throw RustPDF.failure("Image is too large; reduce its dimensions")
    }
    let next = fresh("png")
    try data.write(to: next, options: .atomic)
    return next
  }
  func placeImage(_ file: URL) async {
    guard let image = UIImage(contentsOfFile: file.path) else { return }
    let w = min(pageSize.width * 0.5, 280)
    let h = min(pageSize.height * 0.5, w * image.size.height / image.size.width)
    let width = h * image.size.width / image.size.height
    let x = (pageSize.width - width) / 2
    let y = (pageSize.height - h) / 2
    await mutate(["command": "image", "image": file.path, "rect": [x, y, x + width, y + h]])
  }
}

struct PDFExport: FileDocument {
  static var readableContentTypes: [UTType] { [.pdf] }
  var data: Data
  init(data: Data) { self.data = data }
  init(configuration: ReadConfiguration) throws {
    data = configuration.file.regularFileContents ?? Data()
  }
  func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
    FileWrapper(regularFileWithContents: data)
  }
}
