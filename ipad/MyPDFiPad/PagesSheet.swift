import PDFKit
import SwiftUI
import UniformTypeIdentifiers

/// Touch controls over the desktop Rust page engine. All changes create a revision.
struct PagesSheet: View {
  @EnvironmentObject var model: DocumentModel
  @Environment(\.dismiss) var dismiss
  @State private var selection: Set<Int> = []
  @State private var importing = false
  @State private var deleteConfirm = false
  @State private var crop = false
  @State private var margins: [Double] = [0, 0, 0, 0]
  @State private var error: String?
  var count: Int { model.document?.pageCount ?? 0 }
  var chosen: [Int] { selection.sorted() }
  var body: some View {
    NavigationStack {
      VStack(spacing: 0) {
        HStack {
          Text(selection.isEmpty ? "Select pages · Drag to reorder" : "\(selection.count) selected")
            .font(.subheadline).foregroundStyle(.secondary)
          Spacer()
          Button(selection.count == count ? "Deselect all" : "Select all") {
            selection = selection.count == count ? [] : Set(0..<count)
          }
        }.padding()
        ScrollView {
          LazyVGrid(columns: [GridItem(.adaptive(minimum: 150, maximum: 220))], spacing: 20) {
            ForEach(0..<count, id: \.self) { index in
              if let page = model.document?.page(at: index) {
                Button {
                  if selection.contains(index) {
                    selection.remove(index)
                  } else {
                    selection.insert(index)
                  }
                } label: {
                  VStack(spacing: 8) {
                    PageThumbnail(page: page, revision: model.url?.lastPathComponent ?? "")
                      .frame(height: 200).padding(8).background(
                        .white, in: RoundedRectangle(cornerRadius: 10)
                      )
                      .overlay(
                        RoundedRectangle(cornerRadius: 10).stroke(
                          selection.contains(index) ? Color.blue : Color.gray.opacity(0.2),
                          lineWidth: selection.contains(index) ? 3 : 1))
                    Label(
                      "Page \(index + 1)",
                      systemImage: selection.contains(index) ? "checkmark.circle.fill" : "circle"
                    )
                    .font(.subheadline)
                  }
                }.buttonStyle(.plain).accessibilityLabel(
                  "Page \(index + 1), \(selection.contains(index) ? "selected" : "not selected")"
                )
                .draggable(String(index))
                .dropDestination(for: String.self) { items, _ in
                  guard model.editable, !model.busy, let from = items.first.flatMap(Int.init),
                    from >= 0, from < count, from != index
                  else { return false }
                  var order = Array(0..<count)
                  order.remove(at: from)
                  order.insert(from, at: index)
                  apply("reorder", ["order": order])
                  return true
                }
              }
            }
          }.padding(20)
        }.background(Color(uiColor: .secondarySystemGroupedBackground))
        HStack(spacing: 22) {
          Menu {
            Button("Blank page before selection", systemImage: "doc.badge.plus") {
              insertBlank(at: chosen.first ?? model.page)
            }
            Button("Blank page after selection", systemImage: "doc.badge.plus") {
              insertBlank(at: (chosen.last ?? model.page) + 1)
            }
            Button("Insert or merge PDF…", systemImage: "doc.on.doc") { importing = true }
          } label: {
            Label("Insert", systemImage: "plus")
          }.disabled(!model.editable)
          Menu {
            Button("Rotate left 90°", systemImage: "rotate.left") {
              apply("rotate", ["pages": chosen, "clockwise": false])
            }
            Button("Rotate right 90°", systemImage: "rotate.right") {
              apply("rotate", ["pages": chosen, "clockwise": true])
            }
          } label: {
            Label("Rotate", systemImage: "rotate.right")
          }.disabled(chosen.isEmpty || !model.editable)
          Button {
            crop = true
          } label: {
            Label("Crop", systemImage: "crop")
          }.disabled(chosen.isEmpty || !model.editable)
          Button {
            Task { await extract() }
          } label: {
            Label("Extract", systemImage: "square.and.arrow.up.on.square")
          }.disabled(chosen.isEmpty)
          Spacer()
          Button(role: .destructive) {
            deleteConfirm = true
          } label: {
            Image(systemName: "trash")
          }
          .disabled(chosen.isEmpty || chosen.count == count || !model.editable).accessibilityLabel(
            "Delete selected pages")
        }.padding().background(.bar)
      }.disabled(model.busy)
        .overlay {
          if model.busy {
            ProgressView("Updating pages…").padding(24).background(
              .regularMaterial, in: RoundedRectangle(cornerRadius: 16))
          }
        }
        .navigationTitle("Pages").navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .confirmationAction) {
            Button("Done") { dismiss() }.disabled(model.busy)
          }
        }
        .confirmationDialog(
          "Delete \(selection.count) pages?", isPresented: $deleteConfirm, titleVisibility: .visible
        ) {
          Button("Delete pages", role: .destructive) { apply("delete", ["pages": chosen]) }
          Button("Cancel", role: .cancel) {}
        } message: {
          Text("You can restore them with Undo. The source file is unchanged.")
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.pdf]) { result in
          switch result {
          case .success(let url): Task { await insertPDF(url) }
          case .failure(let failure): error = failure.localizedDescription
          }
        }
        .sheet(isPresented: $crop) {
          NavigationStack {
            Form {
              Section("Crop margins · PDF points") {
                ForEach(0..<4, id: \.self) { i in
                  HStack {
                    Text(["Left", "Top", "Right", "Bottom"][i])
                    Spacer()
                    TextField("0", value: $margins[i], format: .number).keyboardType(.decimalPad)
                      .multilineTextAlignment(.trailing)
                  }
                }
              }
              Text("Cropping hides page edges. It does not redact or remove their contents.").font(
                .footnote)
              Button("Reset crop on selected pages") {
                crop = false
                apply("reset_crop", ["pages": chosen])
              }
            }.navigationTitle("Crop pages").toolbar {
              ToolbarItem(placement: .cancellationAction) { Button("Cancel") { crop = false } }
              ToolbarItem(placement: .confirmationAction) {
                Button("Crop") {
                  crop = false
                  apply("crop", ["pages": chosen, "margins": margins])
                }
              }
            }
          }.presentationDetents([.medium])
        }
        .alert(
          "Page operation",
          isPresented: Binding(
            get: { error != nil || model.error != nil },
            set: {
              if !$0 {
                error = nil
                model.error = nil
              }
            })
        ) {
          Button("OK", role: .cancel) {
            error = nil
            model.error = nil
          }
        } message: {
          Text(error ?? model.error ?? "")
        }
        .onChange(of: model.url) { _, _ in selection = [] }
    }
  }
  func apply(_ action: String, _ parameters: [String: Any]) {
    var command = parameters
    command["command"] = "pages"
    command["action"] = action
    Task { await model.mutate(command) }
  }
  func insertBlank(at: Int) {
    apply("blank", ["at": at, "size": [model.pageSize.width, model.pageSize.height]])
  }
  func insertPDF(_ external: URL) async {
    let scoped = external.startAccessingSecurityScopedResource()
    defer { if scoped { external.stopAccessingSecurityScopedResource() } }
    do {
      guard
        (try external.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) <= 256 * 1024 * 1024
      else { throw RustPDF.failure("Choose a PDF smaller than 256 MB") }
      let copy = model.fresh()
      try FileManager.default.copyItem(at: external, to: copy)
      defer { try? FileManager.default.removeItem(at: copy) }
      await model.mutate([
        "command": "pages", "action": "insert", "at": (chosen.last ?? (count - 1)) + 1,
        "addition": copy.path,
      ])
    } catch { self.error = error.localizedDescription }
  }
  func extract() async {
    await model.extractPages(chosen)
    if model.error == nil { dismiss() }
  }
}

private struct PageThumbnail: View {
  let page: PDFPage
  let revision: String
  @State private var image: UIImage?
  var body: some View {
    Group {
      if let image { Image(uiImage: image).resizable().scaledToFit() } else { ProgressView() }
    }.task(id: revision) {
      image = page.thumbnail(of: CGSize(width: 300, height: 400), for: .cropBox)
    }
  }
}
