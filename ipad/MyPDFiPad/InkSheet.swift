import PencilKit
import SwiftUI

final class PaperCanvas: PKCanvasView {
  var paperSize = CGSize(width: 595, height: 842)
  override func layoutSubviews() {
    super.layoutSubviews()
    guard bounds.width > 0 else { return }
    let scale = bounds.width / paperSize.width
    if abs(zoomScale - scale) > 0.0001 {
      minimumZoomScale = min(minimumZoomScale, scale)
      maximumZoomScale = max(maximumZoomScale, scale)
      setZoomScale(scale, animated: false)
      minimumZoomScale = scale
      maximumZoomScale = scale
    }
    contentSize = paperSize
    contentOffset = .zero
  }
}
struct PencilCanvas: UIViewRepresentable {
  let canvas: PKCanvasView
  let size: CGSize
  let finger: Bool
  let color: UIColor
  let width: CGFloat
  func makeUIView(context: Context) -> PKCanvasView {
    canvas.backgroundColor = .clear
    canvas.isOpaque = false
    canvas.isScrollEnabled = false
    canvas.contentSize = size
    return canvas
  }
  func updateUIView(_ view: PKCanvasView, context: Context) {
    view.drawingPolicy = finger ? .anyInput : .pencilOnly
    view.tool = PKInkingTool(.pen, color: color, width: width)
    (view as? PaperCanvas)?.paperSize = size
    view.setNeedsLayout()
  }
}
struct InkSheet: View {
  @EnvironmentObject var model: DocumentModel
  @Environment(\.dismiss) var dismiss
  let signature: Bool
  @State private var canvas: PKCanvasView = PaperCanvas()
  @AppStorage("fingerDrawing") private var finger = false
  @State private var color = Color(red: 0.08, green: 0.12, blue: 0.22)
  @State private var width = 2.5
  @State private var validation: String?
  var canonical: CGSize { signature ? CGSize(width: 800, height: 300) : model.pageSize }
  var body: some View {
    NavigationStack {
      VStack(spacing: 14) {
        HStack {
          Toggle("Draw with finger", isOn: $finger).fixedSize()
          Spacer()
          ColorPicker("Ink", selection: $color, supportsOpacity: false).fixedSize()
          Slider(value: $width, in: 0.5...12).frame(width: 120).accessibilityLabel("Pen width")
          Text(String(format: "%.1f pt", width)).monospacedDigit()
          Button {
            canvas.undoManager?.undo()
          } label: {
            Image(systemName: "arrow.uturn.backward")
          }.accessibilityLabel("Undo stroke")
          Button("Clear") { canvas.drawing = PKDrawing() }
        }.padding(.horizontal)
        GeometryReader { g in
          let scale = min(
            (g.size.width - 24) / canonical.width, (g.size.height - 24) / canonical.height)
          let display = CGSize(width: canonical.width * scale, height: canonical.height * scale)
          ZStack {
            Color.white
            if !signature, let image = model.thumbnail() {
              Image(uiImage: image).resizable().interpolation(.high)
            }
            PencilCanvas(
              canvas: canvas, size: canonical, finger: finger, color: UIColor(color), width: width)
          }.frame(width: display.width, height: display.height).clipShape(
            RoundedRectangle(cornerRadius: 4)
          ).shadow(color: .black.opacity(0.12), radius: 10, y: 4).position(
            x: g.size.width / 2, y: g.size.height / 2)
        }.background(Color(uiColor: .secondarySystemBackground))
        Text(
          finger
            ? "Draw with your finger or Apple Pencil."
            : "Apple Pencil draws; your hand can rest on the page. Enable finger drawing to use touch."
        ).font(.footnote).foregroundStyle(.secondary)
        if let validation { Text(validation).foregroundStyle(.red).font(.footnote) }
      }.padding(.vertical)
        .navigationTitle(signature ? "Create signature" : "Draw on page \(model.page+1)")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
          ToolbarItem(placement: .confirmationAction) {
            Button(signature ? "Use signature" : "Add drawing") { save() }.bold()
          }
        }
    }.interactiveDismissDisabled(!canvas.drawing.strokes.isEmpty)
  }
  func save() {
    var strokes = canvas.drawing.strokes.map { stroke in
      stroke.path.interpolatedPoints(by: .distance(1.5)).map { point in
        let p = point.location.applying(stroke.transform)
        return [Double(p.x), Double(p.y)]
      }
    }.filter { $0.count >= 2 }
    guard !strokes.isEmpty else {
      validation = "Draw a signature or stroke first."
      return
    }
    guard strokes.flatMap({ $0 }).count <= 60_000 else {
      validation = "This drawing is too detailed. Save a few strokes at a time."
      return
    }
    var r: CGFloat = 0
    var g: CGFloat = 0
    var b: CGFloat = 0
    var a: CGFloat = 0
    UIColor(color).getRed(&r, green: &g, blue: &b, alpha: &a)
    var command: [String: Any] = [
      "command": "ink", "strokes": strokes, "width": width, "color": [r, g, b],
    ]
    if signature {
      let xs = strokes.flatMap { $0.map { $0[0] } }
      let ys = strokes.flatMap { $0.map { $0[1] } }
      let left = xs.min()!
      let top = ys.min()!
      let w = max(1, xs.max()! - left)
      let h = max(1, ys.max()! - top)
      let scale = min(280 / w, 120 / h)
      strokes = strokes.map { $0.map { [($0[0] - left) * scale, ($0[1] - top) * scale] } }
      command["strokes"] = strokes
      command["width"] = max(0.5, width * scale)
      do {
        try JSONSerialization.data(withJSONObject: command).write(
          to: model.signatureURL, options: .atomic)
      } catch {
        validation = error.localizedDescription
        return
      }
      let dx = (model.pageSize.width - 280) / 2
      let dy = (model.pageSize.height - 120) / 2
      command["strokes"] = strokes.map { $0.map { [$0[0] + dx, $0[1] + dy] } }
    }
    dismiss()
    Task { await model.mutate(command) }
  }
}
