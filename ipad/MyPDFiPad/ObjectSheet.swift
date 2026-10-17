import SwiftUI

struct ObjectArtwork: View {
  var object: PDFObject
  var body: some View {
    GeometryReader { g in
      if let data = object.png, let image = UIImage(data: data) {
        Image(uiImage: image).resizable()
      } else if let ink = object.kind["Ink"] as? [String: Any],
        let strokes = ink["strokes"] as? [[[Double]]]
      {
        Canvas { context, size in
          for stroke in strokes {
            var path = Path()
            for (i, p) in stroke.enumerated() {
              let point = CGPoint(x: p[0] * size.width, y: p[1] * size.height)
              if i == 0 { path.move(to: point) } else { path.addLine(to: point) }
            }
            let width =
              (object.style["width"] as? Double ?? 2) * size.width / max(1, object.rect.width)
            context.stroke(
              path, with: .color(object.color),
              style: StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round))
          }
        }
      }
    }.opacity(object.style["opacity"] as? Double ?? 1)
  }
}
struct ObjectSheet: View {
  @EnvironmentObject var model: DocumentModel
  @Environment(\.dismiss) var dismiss
  let object: PDFObject
  @State private var background: UIImage?
  @State private var rect: CGRect = .zero
  @State private var moved = false
  @State private var failure: String?
  @State private var image: ImageImport?
  @GestureState private var drag: CGSize = .zero
  @GestureState private var resize: CGSize = .zero
  var body: some View {
    NavigationStack {
      VStack {
        Text("Drag the object to move it. Drag its corner to resize.").font(.footnote)
          .foregroundStyle(.secondary)
        GeometryReader { g in
          let scale = min(
            (g.size.width - 28) / model.pageSize.width, (g.size.height - 28) / model.pageSize.height
          )
          let size = CGSize(
            width: model.pageSize.width * scale, height: model.pageSize.height * scale)
          let w = max(8, rect.width * scale + resize.width)
          let h = w * rect.height / max(1, rect.width)
          ZStack(alignment: .topLeading) {
            Color.white
            if let background { Image(uiImage: background).resizable() }
            if background != nil {
              ObjectArtwork(object: object).frame(width: w, height: h)
                .overlay { Rectangle().stroke(.blue, lineWidth: 2) }
                .overlay(alignment: .bottomTrailing) {
                  Circle().fill(.white).overlay(Circle().stroke(.blue, lineWidth: 2)).frame(
                    width: 24, height: 24
                  ).offset(x: 12, y: 12)
                    .gesture(
                      DragGesture().updating($resize) { value, state, _ in state = value.translation
                      }.onEnded { value in
                        let width = min(
                          model.pageSize.width - rect.minX,
                          max(8, rect.width + value.translation.width / scale))
                        let height = min(
                          model.pageSize.height - rect.minY,
                          width * rect.height / max(1, rect.width))
                        rect.size = CGSize(
                          width: height * rect.width / max(1, rect.height), height: height)
                        moved = true
                      }
                    )
                    .accessibilityLabel("Resize object")
                }
                .offset(x: rect.minX * scale + drag.width, y: rect.minY * scale + drag.height)
                .gesture(
                  DragGesture().updating($drag) { value, state, _ in state = value.translation }
                    .onEnded { value in
                      rect.origin = CGPoint(
                        x: min(
                          max(0, rect.minX + value.translation.width / scale),
                          max(0, model.pageSize.width - rect.width)),
                        y: min(
                          max(0, rect.minY + value.translation.height / scale),
                          max(0, model.pageSize.height - rect.height)))
                      moved = true
                    }
                )
                .accessibilityLabel(object.label)
            } else if failure == nil {
              ProgressView().frame(width: size.width, height: size.height)
            }
          }.frame(width: size.width, height: size.height).shadow(
            color: .black.opacity(0.12), radius: 8
          ).position(x: g.size.width / 2, y: g.size.height / 2)
        }.background(Color(uiColor: .secondarySystemBackground))
        if let failure { Text(failure).foregroundStyle(.red) }
        if object.isImage {
          Button("Remove background", systemImage: "person.crop.rectangle") {
            do {
              if let data = object.png {
                let url = model.fresh("png")
                try data.write(to: url, options: .atomic)
                image = ImageImport(url: url)
              }
            } catch { failure = error.localizedDescription }
          }.padding(.bottom)
        }
      }
      .navigationTitle(object.label).navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
        ToolbarItem(placement: .confirmationAction) {
          Button("Done") {
            dismiss()
            if moved {
              Task {
                await model.mutate([
                  "command": "place", "id": object.id,
                  "rect": [rect.minX, rect.minY, rect.maxX, rect.maxY],
                ])
              }
            }
          }.bold()
        }
      }
    }.task {
      rect = object.rect
      do { background = try await model.prepareObject(object) } catch {
        failure = error.localizedDescription
      }
    }
    .sheet(item: $image) { item in
      ImageSheet(input: item.url) { url in
        dismiss()
        Task {
          if moved {
            await model.mutate([
              "command": "place", "id": object.id,
              "rect": [rect.minX, rect.minY, rect.maxX, rect.maxY],
            ])
          }
          await model.mutate(["command": "replace_image", "id": object.id, "image": url.path])
        }
      }
    }
    .interactiveDismissDisabled(moved)
  }
}
