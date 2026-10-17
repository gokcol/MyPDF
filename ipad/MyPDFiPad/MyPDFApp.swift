import PDFKit
import PencilKit
import SwiftUI
import UniformTypeIdentifiers

@main struct MyPDFApp: App {
  @StateObject private var model = DocumentModel()
  var body: some Scene {
    WindowGroup {
      ContentView().environmentObject(model).task { await model.recoverInitialDrawings() }.onOpenURL
      { url in Task { await model.importPDF(url) } }
    }
  }
}
struct ImageImport: Identifiable {
  let id = UUID()
  var url: URL
}
struct ContentView: View {
  @EnvironmentObject var model: DocumentModel
  @State private var importing = false
  @State private var imageImport = false
  @State private var exporting = false
  @State private var export: PDFExport?
  @State private var drawing = false
  @State private var signing = false
  @State private var objectList = false
  @State private var image: ImageImport?
  @State private var selected: PDFObject?
  @State private var deleteCandidate: PDFObject?
  @State private var search = ""
  var body: some View {
    NavigationStack {
      Group {
        if let document = model.document {
          VStack(spacing: 0) {
            HStack(spacing: 18) {
              Button {
                drawing = true
              } label: {
                Label("Draw", systemImage: "pencil.tip.crop.circle")
              }
              Menu {
                Button("Draw signature", systemImage: "signature") {
                  signing = true
                  drawing = true
                }
                Button("From image", systemImage: "photo") { imageImport = true }
                if FileManager.default.fileExists(atPath: model.signatureURL.path) {
                  Button("Use saved signature", systemImage: "signature") { reuseSignature() }
                }
              } label: {
                Label("Sign", systemImage: "signature")
              }
              Button {
                imageImport = true
              } label: {
                Label("Image", systemImage: "photo.badge.plus")
              }
              Button {
                objectList = true
              } label: {
                Label("Objects", systemImage: "square.on.circle")
              }
              Spacer()
              Button {
                Task { await model.undo() }
              } label: {
                Image(systemName: "arrow.uturn.backward")
              }.disabled(model.history.isEmpty).accessibilityLabel("Undo")
              Button {
                Task { await model.undo(redo: true) }
              } label: {
                Image(systemName: "arrow.uturn.forward")
              }.disabled(model.future.isEmpty).accessibilityLabel("Redo")
            }.padding().background(.bar).disabled(!model.editable || model.busy)
            PDFReader(document: document, page: $model.page, search: search)
            HStack {
              Button {
                model.page = max(0, model.page - 1)
              } label: {
                Image(systemName: "chevron.left")
              }.disabled(model.page == 0).accessibilityLabel("Previous page")
              Text("\(model.page+1) / \(document.pageCount)").monospacedDigit().frame(minWidth: 90)
              Button {
                model.page = min(document.pageCount - 1, model.page + 1)
              } label: {
                Image(systemName: "chevron.right")
              }.disabled(model.page + 1 >= document.pageCount).accessibilityLabel("Next page")
              Spacer()
              Text(
                model.editable
                  ? "Saved on this iPad · Export a copy to Files"
                  : "Read only · Signed or protected PDF"
              ).font(.footnote).foregroundStyle(.secondary)
            }.padding(12).background(.bar)
          }
        } else {
          home
        }
      }
      .navigationTitle(model.document == nil ? "Home" : model.title).navigationBarTitleDisplayMode(
        .inline
      )
      .toolbar {
        ToolbarItem(placement: .topBarLeading) {
          Button {
            model.document = nil
            model.url = nil
          } label: {
            Image(systemName: "house")
          }.accessibilityLabel("Home").disabled(model.busy)
        }
        ToolbarItemGroup(placement: .topBarTrailing) {
          Button {
            Task { await model.newDocument() }
          } label: {
            Image(systemName: "doc.badge.plus")
          }.accessibilityLabel("New blank PDF")
          Button {
            importing = true
          } label: {
            Image(systemName: "folder")
          }.accessibilityLabel("Open PDF")
          if model.document != nil {
            Button {
              do {
                if let url = model.url {
                  export = PDFExport(data: try Data(contentsOf: url))
                  exporting = true
                }
              } catch { model.error = error.localizedDescription }
            } label: {
              Label("Export", systemImage: "square.and.arrow.up")
            }
          }
        }
      }
      .searchable(text: $search, prompt: "Find text in PDF")
      .overlay {
        if model.busy {
          ProgressView("Working…").padding(24).background(
            .regularMaterial, in: RoundedRectangle(cornerRadius: 18))
        }
      }
      .disabled(model.busy)
    }
    .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf]) { result in
      switch result {
      case .success(let url): Task { await model.importPDF(url) }
      case .failure(let error): model.error = error.localizedDescription
      }
    }
    .fileImporter(isPresented: $imageImport, allowedContentTypes: [.png, .jpeg]) { result in
      do { image = ImageImport(url: try model.importImage(result.get())) } catch {
        model.error = error.localizedDescription
      }
    }
    .fileExporter(
      isPresented: $exporting, document: export, contentType: .pdf,
      defaultFilename: model.title + "-edited"
    ) { result in
      if case .failure(let error) = result { model.error = error.localizedDescription }
    }
    .sheet(isPresented: $drawing, onDismiss: { signing = false }) { InkSheet(signature: signing) }
    .sheet(item: $image) { item in
      ImageSheet(input: item.url) { file in Task { await model.placeImage(file) } }
    }
    .sheet(isPresented: $objectList) {
      NavigationStack {
        List(model.objects.filter { $0.page == model.page }) { object in
          HStack {
            Image(systemName: object.isImage ? "photo" : "pencil.tip").foregroundStyle(.blue)
            Text(object.label)
            Spacer()
            if object.canMove {
              Button("Edit") {
                objectList = false
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { selected = object }
              }.buttonStyle(.borderless)
            }
            Button(role: .destructive) {
              deleteCandidate = object
            } label: {
              Image(systemName: "trash")
            }.buttonStyle(.borderless).accessibilityLabel("Delete object")
          }.padding(.vertical, 8)
        }
        .overlay {
          if model.objects.filter({ $0.page == model.page }).isEmpty {
            ContentUnavailableView(
              "No MyPDF objects on this page", systemImage: "pencil.tip",
              description: Text("Draw or add an image to create an editable object."))
          }
        }
        .navigationTitle("Objects · Page \(model.page+1)").toolbar {
          Button("Done") { objectList = false }
        }
        .confirmationDialog(
          "Delete this object?",
          isPresented: Binding(
            get: { deleteCandidate != nil }, set: { if !$0 { deleteCandidate = nil } }),
          titleVisibility: .visible
        ) {
          Button("Delete", role: .destructive) {
            if let target = deleteCandidate {
              deleteCandidate = nil
              objectList = false
              Task { await model.mutate(["command": "delete", "id": target.id]) }
            }
          }
          Button("Cancel", role: .cancel) { deleteCandidate = nil }
        } message: {
          Text("You can restore it with Undo.")
        }
      }
    }
    .sheet(item: $selected) { object in ObjectSheet(object: object) }
    .alert(
      "MyPDF",
      isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
    ) {
      Button("OK", role: .cancel) { model.error = nil }
    } message: {
      Text(model.error ?? "")
    }
  }
  var home: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 28) {
        Text("Your PDFs. Your Pencil.").font(.largeTitle.bold())
        HStack(spacing: 20) {
          homeCard("Open PDF", "folder.fill", "Choose from Files") { importing = true }
          homeCard("Blank PDF", "doc.badge.plus", "Start with a clean page") {
            Task { await model.newDocument() }
          }
        }
        Text("On this iPad").font(.title2.bold())
        if model.library.isEmpty {
          Text("Imported documents are copied here. Your originals stay unchanged.")
            .foregroundStyle(.secondary)
        }
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 220))], spacing: 18) {
          ForEach(model.library) { item in
            Button {
              Task { await model.activate(item) }
            } label: {
              VStack(alignment: .leading, spacing: 12) {
                Image(systemName: "doc.richtext").font(.system(size: 40)).foregroundStyle(.blue)
                Text(item.name).font(.headline).lineLimit(2)
                Text(item.modified, style: .date).font(.caption).foregroundStyle(.secondary)
              }.frame(maxWidth: .infinity, minHeight: 140, alignment: .leading).padding()
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 20))
            }.buttonStyle(.plain)
          }
        }
        Text("iPad development preview · Native PDFKit + PencilKit · Rust PDF engine").font(
          .footnote
        ).foregroundStyle(.secondary)
      }.padding(32).frame(maxWidth: 1100)
    }.background(Color(uiColor: .systemGroupedBackground))
  }
  func homeCard(_ title: String, _ icon: String, _ detail: String, action: @escaping () -> Void)
    -> some View
  {
    Button(action: action) {
      VStack(alignment: .leading, spacing: 14) {
        Image(systemName: icon).font(.system(size: 36))
        Text(title).font(.title2.bold())
        Text(detail).font(.subheadline)
      }.frame(maxWidth: .infinity, alignment: .leading).padding(28).background(
        Color.blue.opacity(0.09), in: RoundedRectangle(cornerRadius: 24))
    }.buttonStyle(.plain).foregroundStyle(.blue)
  }
  func reuseSignature() {
    do {
      var command =
        try JSONSerialization.jsonObject(with: Data(contentsOf: model.signatureURL))
        as? [String: Any] ?? [:]
      if let strokes = command["strokes"] as? [[[Double]]] {
        let dx = (model.pageSize.width - 280) / 2
        let dy = (model.pageSize.height - 120) / 2
        command["strokes"] = strokes.map { $0.map { [$0[0] + dx, $0[1] + dy] } }
      }
      Task { await model.mutate(command) }
    } catch { model.error = error.localizedDescription }
  }
}

struct PDFReader: UIViewRepresentable {
  let document: PDFDocument
  @Binding var page: Int
  let search: String
  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeUIView(context: Context) -> PDFView {
    let view = PDFView()
    view.autoScales = true
    view.displayMode = .singlePageContinuous
    view.displayDirection = .vertical
    view.backgroundColor = .secondarySystemBackground
    NotificationCenter.default.addObserver(
      context.coordinator, selector: #selector(Coordinator.changed(_:)), name: .PDFViewPageChanged,
      object: view)
    return view
  }
  func updateUIView(_ view: PDFView, context: Context) {
    context.coordinator.parent = self
    if view.document !== document {
      view.document = document
      view.autoScales = true
    }
    if let target = document.page(at: page), view.currentPage !== target { view.go(to: target) }
    if search != context.coordinator.lastSearch {
      context.coordinator.lastSearch = search
      view.highlightedSelections =
        search.isEmpty ? nil : document.findString(search, withOptions: .caseInsensitive)
    }
  }
  final class Coordinator: NSObject {
    var parent: PDFReader
    var lastSearch = ""
    init(_ parent: PDFReader) { self.parent = parent }
    @objc func changed(_ note: Notification) {
      guard let view = note.object as? PDFView, let p = view.currentPage, let d = view.document
      else { return }
      let index = d.index(for: p)
      if index != parent.page { DispatchQueue.main.async { self.parent.page = index } }
    }
    deinit { NotificationCenter.default.removeObserver(self) }
  }
}
