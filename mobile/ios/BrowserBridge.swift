// Device approval uses Safari; no wallet signing or purchases are performed here.
import Foundation
import UIKit

private final class BrowserResult: @unchecked Sendable {
    static let shared = BrowserResult()
    private let lock = NSLock()
    private var failed = false
    func set(_ value: Bool) { lock.lock(); defer { lock.unlock() }; failed = value }
    func get() -> Bool { lock.lock(); defer { lock.unlock() }; return failed }
}

@_cdecl("omoba_browser_open")
public func omobaBrowserOpen(_ pointer: UnsafePointer<CChar>) {
    let text = String(cString: pointer)
    guard let url = URL(string: text), ["https", "http"].contains(url.scheme ?? ""),
          url.user == nil, url.password == nil else {
        BrowserResult.shared.set(true); return
    }
    BrowserResult.shared.set(false)
    DispatchQueue.main.async {
        UIApplication.shared.open(url, options: [:]) { accepted in
            BrowserResult.shared.set(!accepted)
        }
    }
}

@_cdecl("omoba_browser_failed")
public func omobaBrowserFailed() -> Bool { BrowserResult.shared.get() }
