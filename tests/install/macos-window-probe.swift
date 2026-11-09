// Read only this child application's on-screen window metadata, never other apps.
import AppKit
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2, let pid = Int(CommandLine.arguments[1]) else {
    fatalError("Expected the owned child PID")
}
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
    as? [[String: Any]] ?? []
let owned = windows.compactMap { window -> [String: Any]? in
    guard window[kCGWindowOwnerPID as String] as? Int == pid,
          let bounds = window[kCGWindowBounds as String] as? [String: Any],
          let x = bounds["X"] as? Double, let y = bounds["Y"] as? Double,
          let width = bounds["Width"] as? Double, let height = bounds["Height"] as? Double,
          width >= 200, height >= 100 else { return nil }
    return ["x": x, "y": y, "width": width, "height": height]
}
guard let screen = NSScreen.screens.first else { fatalError("No primary display work area") }
let frame = screen.visibleFrame
let data = try JSONSerialization.data(withJSONObject: ["pid": pid, "windows": owned,
    // AppKit screen coordinates start at the lower left of the primary
    // screen, while CoreGraphics window coordinates start at the upper left.
    "work_area": ["x": Double(frame.minX), "y": Double(screen.frame.maxY - frame.maxY),
                  "width": Double(frame.width), "height": Double(frame.height)],
    "scale_factor": Double(screen.backingScaleFactor)])
FileHandle.standardOutput.write(data)
if owned.isEmpty { exit(1) }
