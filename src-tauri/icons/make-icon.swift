import AppKit
guard CommandLine.arguments.count == 3,
      let source = NSImage(contentsOfFile: CommandLine.arguments[1]),
      source.size.width > 0, source.size.width == source.size.height else {
    fputs("Usage: swift make-icon.swift <square source.png> <output.png>\n", stderr)
    exit(1)
}
let size=NSSize(width:1024,height:1024)
let bitmap=NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:1024,pixelsHigh:1024,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0)!
NSGraphicsContext.saveGraphicsState()
let context=NSGraphicsContext(bitmapImageRep:bitmap)!
NSGraphicsContext.current=context
context.imageInterpolation = .high
NSBezierPath(roundedRect:NSRect(origin:.zero,size:size),xRadius:215,yRadius:215).addClip()
source.draw(in:NSRect(origin:.zero,size:size))
NSGraphicsContext.restoreGraphicsState()
let data=bitmap.representation(using:.png,properties:[:])!
try data.write(to:URL(fileURLWithPath:CommandLine.arguments[2]),options:.atomic)
