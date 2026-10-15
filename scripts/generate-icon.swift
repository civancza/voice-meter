// Regenerate the checked-in artwork using only macOS AppKit and iconutil.
import AppKit

func color(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 255) / 255,
        green: CGFloat((hex >> 8) & 255) / 255,
        blue: CGFloat(hex & 255) / 255,
        alpha: alpha
    )
}

func rounded(_ rect: NSRect, _ radius: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
}

func gradient(_ path: NSBezierPath, _ top: UInt32, _ bottom: UInt32) {
    NSGradient(starting: color(bottom), ending: color(top))!.draw(in: path, angle: 90)
}

func drawIcon() {
    let tile = rounded(NSRect(x: 80, y: 86, width: 864, height: 864), 194)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = color(0x000C0A, alpha: 0.35)
    shadow.shadowBlurRadius = 25
    shadow.shadowOffset = NSSize(width: 0, height: -16)
    shadow.set()
    gradient(tile, 0x263F3B, 0x0A1817)
    NSGraphicsContext.restoreGraphicsState()

    NSGraphicsContext.saveGraphicsState()
    tile.addClip()
    let glow = NSGradient(colors: [color(0x86FFD4, alpha: 0.16), color(0x86FFD4, alpha: 0)])!
    glow.draw(
        fromCenter: NSPoint(x: 300, y: 900), radius: 0,
        toCenter: NSPoint(x: 300, y: 900), radius: 760,
        options: [.drawsAfterEndingLocation]
    )
    NSGraphicsContext.restoreGraphicsState()

    color(0xD3FFF0, alpha: 0.2).setStroke()
    tile.lineWidth = 2
    tile.stroke()
    let inset = rounded(NSRect(x: 91, y: 97, width: 842, height: 842), 184)
    color(0xE6FFF6, alpha: 0.05).setStroke()
    inset.lineWidth = 2
    inset.stroke()

    let glass = rounded(NSRect(x: 156, y: 222, width: 712, height: 586), 110)
    gradient(glass, 0x243E37, 0x142723)
    color(0xD0FFE6, alpha: 0.12).setStroke()
    glass.lineWidth = 2
    glass.stroke()

    let heights: [CGFloat] = [130, 250, 390, 502, 360, 240, 124]
    let colors: [(UInt32, UInt32)] = [
        (0x9CFFD8, 0x42DCA4), (0x9CFFD8, 0x42DCA4),
        (0x9CFFD8, 0x42DCA4), (0xB5FFDD, 0x52EAB0),
        (0xC3F59C, 0x8AD776), (0xFFE2A0, 0xF3B851),
        (0xFFAD9D, 0xF47764),
    ]
    for (index, height) in heights.enumerated() {
        let rect = NSRect(x: 210 + CGFloat(index) * 88, y: 515 - height / 2, width: 64, height: height)
        let bar = rounded(rect, 32)
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = color(colors[index].1, alpha: 0.25)
        shadow.shadowBlurRadius = 20
        shadow.shadowOffset = .zero
        shadow.set()
        gradient(bar, colors[index].0, colors[index].1)
        NSGraphicsContext.restoreGraphicsState()
        color(0xFFFFFF, alpha: 0.24).setStroke()
        bar.lineWidth = 1.5
        bar.stroke()
    }
}

func png(size: Int) throws -> Data {
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    let transform = NSAffineTransform()
    transform.scale(by: CGFloat(size) / 1024)
    transform.concat()
    drawIcon()
    NSGraphicsContext.restoreGraphicsState()
    guard let data = bitmap.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "VoiceMeterIcon", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "Cannot encode \(size)px icon",
        ])
    }
    return data
}

let files = FileManager.default
let output = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? "assets", isDirectory: true)
try files.createDirectory(at: output, withIntermediateDirectories: true)
let temporary = files.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
let iconset = temporary.appendingPathComponent("VoiceMeter.iconset", isDirectory: true)
try files.createDirectory(at: iconset, withIntermediateDirectories: true)
defer {
    do {
        try files.removeItem(at: temporary)
    } catch {
        fputs("Cannot remove temporary iconset: \(error)\n", stderr)
    }
}
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let suffix = scale == 2 ? "@2x" : ""
        try png(size: points * scale).write(
            to: iconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png")
        )
    }
}
try png(size: 1024).write(to: output.appendingPathComponent("VoiceMeter.png"))
let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconset.path, "-o", output.appendingPathComponent("VoiceMeter.icns").path]
try process.run()
process.waitUntilExit()
guard process.terminationStatus == 0 else {
    throw NSError(domain: "VoiceMeterIcon", code: Int(process.terminationStatus), userInfo: [
        NSLocalizedDescriptionKey: "iconutil failed",
    ])
}
print("Generated \(output.path)/VoiceMeter.{png,icns}")
