import SwiftUI

struct Checkerboard: View {
  var body: some View {
    Canvas { ctx, size in
      ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
      for y in stride(from: 0.0, to: size.height, by: 16) {
        for x in stride(from: 0.0, to: size.width, by: 16)
        where (Int(x / 16) + Int(y / 16)) % 2 == 0 {
          ctx.fill(
            Path(CGRect(x: x, y: y, width: 16, height: 16)), with: .color(.gray.opacity(0.18)))
        }
      }
    }
  }
}
struct ImageSheet: View {
  @EnvironmentObject var model: DocumentModel
  @Environment(\.dismiss) var dismiss
  let input: URL
  var use: (URL) -> Void
  @State private var transparent = false
  @State private var tolerance = 24.0
  @State private var edgeOnly = false
  @State private var output: URL?
  @State private var updating = false
  @State private var generation = 0
  @State private var error: String?
  var body: some View {
    NavigationStack {
      VStack(spacing: 22) {
        ZStack {
          Checkerboard()
          if let image = UIImage(contentsOfFile: (transparent ? output ?? input : input).path) {
            Image(uiImage: image).resizable().scaledToFit().padding(20)
          }
          if updating { ProgressView().padding(20).background(.regularMaterial, in: Capsule()) }
        }.frame(maxHeight: 360).clipShape(RoundedRectangle(cornerRadius: 18))
        Toggle("Remove white background", isOn: $transparent).onChange(of: transparent) { _, _ in
          update()
        }
        if transparent {
          HStack {
            Text("Tolerance")
            Slider(value: $tolerance, in: 0...120, step: 1).onChange(of: tolerance) { _, _ in
              update()
            }
            Text("\(Int(tolerance))").monospacedDigit()
          }
          Toggle("Only remove areas connected to an edge", isOn: $edgeOnly).onChange(of: edgeOnly) {
            _, _ in update()
          }
        }
        Text(
          "Best for signatures, scans, and images on white paper. Your source image is preserved."
        ).foregroundStyle(.secondary).font(.footnote)
        if let error { Text(error).foregroundStyle(.red) }
        Spacer()
      }.padding(24)
        .navigationTitle("Image & transparency").navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
          ToolbarItem(placement: .confirmationAction) {
            Button("Use image") {
              use(transparent ? output! : input)
              dismiss()
            }.bold().disabled(updating || (transparent && (output == nil || error != nil)))
          }
        }
    }
  }
  func update() {
    generation += 1
    let token = generation
    guard transparent else {
      updating = false
      return
    }
    updating = true
    error = nil
    let value = Int(tolerance)
    let edges = edgeOnly
    let result = model.fresh("png")
    Task {
      try? await Task.sleep(for: .milliseconds(200))
      guard token == generation else { return }
      do {
        _ = try await RustPDF.run([
          "command": "background", "input": input.path, "output": result.path, "tolerance": value,
          "edgesOnly": edges,
        ])
        if token == generation {
          output = result
          updating = false
        }
      } catch {
        if token == generation {
          self.error = error.localizedDescription
          updating = false
        }
      }
    }
  }
}
