import PDFKit
import PencilKit
import SwiftUI
import XCTest

@testable import MyPDFiPad

final class NativeTests: XCTestCase {
  func testRustBridgeCreatesEditableDrawingAndTransparentSignature() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let blank = root.appendingPathComponent("blank.pdf")
    let ink = root.appendingPathComponent("ink.pdf")
    _ = try RustPDF.call(["command": "blank", "output": blank.path])
    _ = try RustPDF.call([
      "command": "ink", "input": blank.path, "output": ink.path, "page": 0,
      "color": [0.1, 0.2, 0.5], "width": 2.0, "strokes": [[[20, 30], [40, 60], [80, 25]]],
    ])
    let result = try RustPDF.call(["command": "annotations", "input": ink.path])
    let marks = try XCTUnwrap(result["annotations"] as? [[String: Any]])
    XCTAssertEqual(marks.count, 1)
    let drawing = PDFObject(raw: marks[0])
    XCTAssertTrue(drawing.canMove)
    XCTAssertNotNil(PDFDocument(url: ink)?.page(at: 0))
    let original = try Data(contentsOf: blank)
    let moved = root.appendingPathComponent("moved.pdf")
    _ = try RustPDF.call([
      "command": "place", "input": ink.path, "output": moved.path, "page": 0, "id": drawing.id,
      "rect": [100, 100, 220, 170],
    ])
    XCTAssertEqual(try Data(contentsOf: blank), original)
    let pixels: [UInt8] = [
      255, 255, 255, 255, 20, 30, 70, 255, 20, 30, 70, 255, 255, 255, 255, 255,
    ]
    let provider = CGDataProvider(data: Data(pixels) as CFData)!
    let image = CGImage(
      width: 2, height: 2, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: 8,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue), provider: provider,
      decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    let source = root.appendingPathComponent("signature.png")
    let clear = root.appendingPathComponent("clear.png")
    try UIImage(cgImage: image).pngData()!.write(to: source)
    _ = try RustPDF.call(["command": "background", "input": source.path, "output": clear.path])
    let data = try Data(contentsOf: clear)
    let decoded = try XCTUnwrap(UIImage(data: data)?.cgImage)
    var rgba = [UInt8](repeating: 0, count: 16)
    let context = CGContext(
      data: &rgba, width: 2, height: 2, bitsPerComponent: 8, bytesPerRow: 8,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.draw(decoded, in: CGRect(x: 0, y: 0, width: 2, height: 2))
    XCTAssertEqual([rgba[3], rgba[7], rgba[11], rgba[15]].sorted(), [0, 0, 255, 255])
    let placed = root.appendingPathComponent("placed.pdf")
    _ = try RustPDF.call([
      "command": "image", "input": moved.path, "output": placed.path, "page": 0,
      "image": clear.path, "rect": [50, 150, 150, 250],
    ])
    let saved =
      try RustPDF.call(["command": "annotations", "input": placed.path])["annotations"]
      as! [[String: Any]]
    XCTAssertEqual(saved.count, 2)
    XCTAssertEqual(PDFObject(raw: saved[1]).png, data)
    XCTAssertThrowsError(try RustPDF.call(["command": "blank", "output": blank.path]))
    XCTAssertEqual(try Data(contentsOf: blank), original)
  }
  func testPageOrganizerUsesDesktopEngineAndPreservesDrawing() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    func file(_ name: String) -> String { root.appendingPathComponent(name + ".pdf").path }
    _ = try RustPDF.call(["command": "blank", "output": file("blank")])
    _ = try RustPDF.call([
      "command": "ink", "input": file("blank"), "output": file("drawing"), "page": 0,
      "color": [0.1, 0.2, 0.5], "width": 2.0, "strokes": [[[30, 40], [90, 100], [130, 45]]],
    ])
    let original = try Data(contentsOf: URL(fileURLWithPath: file("drawing")))
    func pages(_ input: String, _ output: String, _ action: String, _ params: [String: Any]) throws
    {
      var args = params
      args["command"] = "pages"
      args["input"] = file(input)
      args["output"] = file(output)
      args["action"] = action
      _ = try RustPDF.call(args)
    }
    try pages("drawing", "inserted", "blank", ["at": 1, "size": [595, 842]])
    try pages("inserted", "rotated", "rotate", ["pages": [0], "clockwise": true])
    try pages("rotated", "reordered", "reorder", ["order": [1, 0]])
    let info = try RustPDF.call(["command": "annotations", "input": file("reordered")])
    let marks = try XCTUnwrap(info["annotations"] as? [[String: Any]])
    XCTAssertEqual(marks.count, 1)
    XCTAssertEqual(PDFObject(raw: marks[0]).page, 1)
    let sizes = try XCTUnwrap(info["pages"] as? [[Double]])
    XCTAssertEqual(sizes[1], [842, 595])
    try pages("reordered", "cropped", "crop", ["pages": [1], "margins": [10, 10, 10, 10]])
    try pages("cropped", "extracted", "extract", ["pages": [1]])
    XCTAssertEqual(PDFDocument(url: URL(fileURLWithPath: file("extracted")))?.pageCount, 1)
    let extracted = try RustPDF.call(["command": "annotations", "input": file("extracted")])
    XCTAssertEqual((extracted["annotations"] as? [[String: Any]])?.count, 1)
    try pages("cropped", "deleted", "delete", ["pages": [0]])
    try pages("deleted", "merged", "insert", ["at": 1, "addition": file("blank")])
    XCTAssertEqual(PDFDocument(url: URL(fileURLWithPath: file("merged")))?.pageCount, 2)
    XCTAssertThrowsError(try pages("merged", "invalid", "delete", ["pages": [0, 1]]))
    XCTAssertFalse(FileManager.default.fileExists(atPath: file("invalid")))
    XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: file("drawing"))), original)
    XCTAssertThrowsError(try pages("merged", "drawing", "extract", ["pages": [0]]))
  }
  @MainActor func testPencilCoordinatesSurviveCanvasResize() {
    let canvas = PaperCanvas(frame: CGRect(x: 0, y: 0, width: 300, height: 400))
    canvas.paperSize = CGSize(width: 600, height: 800)
    let points = [
      PKStrokePoint(
        location: CGPoint(x: 100, y: 200), timeOffset: 0, size: CGSize(width: 3, height: 3),
        opacity: 1, force: 1, azimuth: 0, altitude: 1),
      PKStrokePoint(
        location: CGPoint(x: 200, y: 300), timeOffset: 0.1, size: CGSize(width: 3, height: 3),
        opacity: 1, force: 1, azimuth: 0, altitude: 1),
    ]
    canvas.drawing = PKDrawing(strokes: [
      PKStroke(
        ink: PKInk(.pen, color: .black),
        path: PKStrokePath(controlPoints: points, creationDate: Date()))
    ])
    canvas.setNeedsLayout()
    canvas.layoutIfNeeded()
    XCTAssertEqual(canvas.zoomScale, 0.5, accuracy: 0.001)
    canvas.frame = CGRect(x: 0, y: 0, width: 600, height: 800)
    canvas.setNeedsLayout()
    canvas.layoutIfNeeded()
    XCTAssertEqual(canvas.zoomScale, 1, accuracy: 0.001)
    XCTAssertEqual(canvas.drawing.strokes[0].path[0].location.x, 100, accuracy: 0.01)
  }
}
