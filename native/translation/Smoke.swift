// Read-only native integration smoke: discovery and pair checks never request a
// model download and never receive a PDF. Run via smoke.sh on a logged-in Mac.
import AppKit
import Foundation

private var awaiting: Set<UInt64> = [1, 2, 3]
private var seen: Set<UInt64> = []
private var failures = 0
private let callback: @convention(c) (UInt64, UnsafePointer<CChar>) -> Void = { id, pointer in
    let text = String(cString: pointer)
    guard let json = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
          let event = json["event"] as? String else { exit(2) }
    switch event {
    case "languages":
        let languages = json["value"] as? [[String: String]] ?? []
        let codes = Set(languages.compactMap { $0["id"] })
        print("Supported language count: \(languages.count); English: \(codes.contains("en")); German: \(codes.contains("de")); Turkish: \(codes.contains("tr"))")
        if languages.isEmpty { failures += 1 }
        seen.insert(id)
    case "availability":
        print("Pair \(id == 2 ? "de → en" : "tr → en"): \(json["value"] ?? "unknown")")
        seen.insert(id)
    case "translated":
        let value = json["value"] as? [String: Any] ?? [:]
        let translated = value["text"] as? String ?? ""
        if translated.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { failures += 1 }
        print("Synthetic translation \(id): \(translated)")
        seen.insert(id)
    case "failed", "cancelled":
        print("Read-only request \(id) did not complete")
        failures += 1
        awaiting.remove(id)
    case "finished":
        if !seen.contains(id) { failures += 1 }
        awaiting.remove(id)
    default: break
    }
    if awaiting.isEmpty { exit(failures == 0 ? 0 : 1) }
}

@main
struct Smoke {
    static func main() {
        guard translationAvailable() else {
            print("Requires macOS 26 or later")
            exit(1)
        }
        _ = NSApplication.shared
        var requests = [
            #"{"operation":"languages"}"#,
            #"{"operation":"check","source":"de","target":"en"}"#,
            #"{"operation":"check","source":"tr","target":"en"}"#,
        ]
        if CommandLine.arguments.contains("--translate") {
            requests += [
                #"{"operation":"translate","source":"de","target":"en","texts":["Guten Morgen. Dieses Dokument bleibt auf meinem Computer."]}"#,
                #"{"operation":"translate","source":"en","target":"de","texts":["Good morning. This document stays on my computer."]}"#,
                #"{"operation":"translate","source":"tr","target":"en","texts":["Günaydın. Bu belge bilgisayarımda kalır."]}"#,
                #"{"operation":"translate","source":"en","target":"tr","texts":["Good morning. This document stays on my computer."]}"#,
            ]
            awaiting = Set((1...requests.count).map(UInt64.init))
        }
        for (index, request) in requests.enumerated() {
            request.withCString { translationStart(UInt64(index + 1), $0, callback) }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 45) {
            print("Timed out waiting for Apple's local translation service")
            exit(2)
        }
        NSApplication.shared.run()
    }
}
