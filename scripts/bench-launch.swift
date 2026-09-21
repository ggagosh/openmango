// Launch benchmark: time from process start to first window on screen, and memory once settled.
// Runs against a throwaway profile (HOME is redirected), so saved connections are untouched.
// Run 1 is a first-ever launch; later runs reuse the profile.
//
//   swift scripts/bench-launch.swift /Applications/OpenMango.app/Contents/MacOS/OpenMango 6
//
// With a collection name, the profile holds a saved session on mongodb://localhost:27018 (seeded
// by scripts/bench-seed.js), so the app reconnects and reopens bench.<collection> on launch:
//
//   swift scripts/bench-launch.swift /Applications/OpenMango.app/Contents/MacOS/OpenMango 3 orders

import AppKit

let binary = CommandLine.arguments[1]
let runs = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2])! : 6
let collection = CommandLine.arguments.count > 3 ? CommandLine.arguments[3] : nil
let home = FileManager.default.temporaryDirectory.appendingPathComponent("openmango-bench-\(UUID())")
let library = home.appendingPathComponent("Library")
try FileManager.default.createDirectory(at: library, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: home) }
// macOS finds the login keychain through HOME; without this link the app's first keychain
// call raises a "Keychain Not Found" dialog.
try FileManager.default.createSymbolicLink(
    at: library.appendingPathComponent("Keychains"),
    withDestinationURL: FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Keychains"))

func shell(_ command: String) -> String {
    let process = Process()
    let pipe = Pipe()
    process.executableURL = URL(fileURLWithPath: "/bin/sh")
    process.arguments = ["-c", command]
    process.standardOutput = pipe
    try? process.run()
    process.waitUntilExit()
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    return String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
}

func writeSession(_ collection: String) throws {
    let profile = library.appendingPathComponent("Application Support/openmango")
    try FileManager.default.createDirectory(at: profile, withIntermediateDirectories: true)
    let id = "11111111-1111-4111-8111-111111111111"
    let connections = """
        [{"id": "\(id)", "name": "bench", "uri": "mongodb://localhost:27018", "last_connected": null}]
        """
    let workspace = """
        {"last_connection_id": "\(id)", "selected_database": "bench",
         "selected_collection": "\(collection)", "active_tab": 0, "expanded_nodes": [],
         "window_state": null,
         "open_tabs": [{"database": "bench", "collection": "\(collection)"}]}
        """
    try connections.write(
        to: profile.appendingPathComponent("connections.json"), atomically: true, encoding: .utf8)
    try workspace.write(
        to: profile.appendingPathComponent("workspace.json"), atomically: true, encoding: .utf8)
}

for run in 1...runs {
    if let collection { try writeSession(collection) }
    let app = Process()
    app.executableURL = URL(fileURLWithPath: binary)
    app.environment = ["HOME": home.path, "PATH": "/usr/bin:/bin"]
    app.standardOutput = FileHandle.nullDevice
    app.standardError = FileHandle.nullDevice
    let start = DispatchTime.now().uptimeNanoseconds
    try app.run()

    var windowMs: Double?
    while windowMs == nil, app.isRunning {
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - start) / 1e6
        if elapsed > 30_000 { break }
        let windows =
            CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
        let visible = windows.contains {
            ($0[kCGWindowOwnerPID as String] as? Int32) == app.processIdentifier
                && ($0[kCGWindowLayer as String] as? Int) == 0
        }
        if visible { windowMs = elapsed } else { usleep(2_000) }
    }

    sleep(collection == nil ? 8 : 15)  // let startup, and any reconnect and load, settle
    let onScreen =
        CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    if onScreen.contains(where: { ($0[kCGWindowOwnerName as String] as? String) == "SecurityAgent" }) {
        print("run \(run): a system security dialog is on screen; this run is not clean")
    }
    var memory = ""
    for _ in 0..<3 where memory.isEmpty {  // top occasionally prints nothing
        memory = shell("top -l 1 -pid \(app.processIdentifier) -stats mem | tail -1")
    }
    app.terminate()
    app.waitUntilExit()
    let shown = windowMs.map { String(format: "%.0f ms", $0) } ?? "no window within 30 s"
    print("run \(run): window \(shown), memory \(memory) (\(collection ?? "no connection"))")
    sleep(2)
}
