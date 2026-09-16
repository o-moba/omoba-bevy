// Original code-drawn beta icon, under the repository's source license.
// Run: swift render_beta_icon.swift /absolute/output/AppIcon.png
import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else { fatalError("Supply a PNG output path") }
let canvas = CGContext(data: nil, width: 1024, height: 1024, bitsPerComponent: 8,
    bytesPerRow: 4096, space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(cgContext: canvas, flipped: false)
func color(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> NSColor {
    NSColor(srgbRed: r / 255, green: g / 255, blue: b / 255, alpha: 1)
}
let bounds = NSRect(x: 0, y: 0, width: 1024, height: 1024)
NSGradient(starting: color(26, 21, 53), ending: color(3, 3, 8))!.draw(in: bounds, angle: -60)
// An open arena ring and opposing bases; colors match the Omoba landing page.
let ring = NSBezierPath()
ring.appendArc(withCenter: NSPoint(x: 512, y: 544), radius: 260,
               startAngle: 35, endAngle: 325, clockwise: false)
ring.lineWidth = 68
ring.lineCapStyle = .round
color(34, 211, 238).setStroke()
ring.stroke()
let lane = NSBezierPath()
lane.move(to: NSPoint(x: 378, y: 410))
lane.line(to: NSPoint(x: 646, y: 678))
lane.lineWidth = 24
color(240, 193, 75).setStroke()
lane.stroke()
for (x, y) in [(378.0, 410.0), (646.0, 678.0)] {
    let base = NSBezierPath()
    base.move(to: NSPoint(x: x, y: y + 48))
    base.line(to: NSPoint(x: x + 48, y: y))
    base.line(to: NSPoint(x: x, y: y - 48))
    base.line(to: NSPoint(x: x - 48, y: y))
    base.close()
    color(240, 193, 75).setFill()
    base.fill()
}
let label = "OMOBA" as NSString
let attributes: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 72, weight: .bold),
    .foregroundColor: NSColor.white, .kern: 10]
let size = label.size(withAttributes: attributes)
label.draw(at: NSPoint(x: (1024 - size.width) / 2, y: 126), withAttributes: attributes)
NSGraphicsContext.restoreGraphicsState()
let bitmap = NSBitmapImageRep(cgImage: canvas.makeImage()!)
try bitmap.representation(using: .png, properties: [:])!
    .write(to: URL(fileURLWithPath: CommandLine.arguments[1]), options: .withoutOverwriting)
