// StoreKit owns payment UI. Rust only receives signed transaction data.
import Foundation
import StoreKit

private final class EventQueue: @unchecked Sendable {
    static let shared = EventQueue()
    private let lock = NSLock()
    private var values: [Data] = []
    func push(_ value: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: value), data.count < 60000 else { return }
        lock.lock(); defer { lock.unlock() }
        // Unacknowledged transactions are replayed by StoreKit on the next launch.
        if values.count < 32 { values.append(data) }
    }
    func copyNext(_ buffer: UnsafeMutablePointer<CChar>, capacity: Int) -> Int32 {
        lock.lock(); defer { lock.unlock() }
        guard let data = values.first, data.count + 1 <= capacity else { return 0 }
        data.withUnsafeBytes { bytes in
            if let address = bytes.baseAddress { memcpy(buffer, address, data.count) }
        }
        buffer[data.count] = 0
        values.removeFirst()
        return Int32(data.count)
    }
}

@MainActor private final class SupporterStore {
    static let shared = SupporterStore()
    private var product: Product?
    private var productID: String?
    private var listener: Task<Void, Never>?
    private var unfinished: [String: Transaction] = [:]
    private var purchasing = false

    func accept(_ result: VerificationResult<Transaction>) {
        guard case .verified(let transaction) = result,
              transaction.productID == productID else { return }
        let id = String(transaction.id)
        unfinished[id] = transaction
        EventQueue.shared.push(["kind": "transaction", "transaction_id": id,
                                "signed_payload": result.jwsRepresentation])
    }

    func configure(_ id: String) async {
        productID = id
        if listener == nil {
            listener = Task { [weak self] in
                for await update in Transaction.updates { self?.accept(update) }
            }
        }
        do {
            product = try await Product.products(for: [id]).first
            guard let product, product.type == .autoRenewable,
                  let subscription = product.subscription else {
                EventQueue.shared.push(["kind": "error", "message": "Supporter is not available in this App Store build."])
                return
            }
            // Only the agreed monthly product may be sold by this screen.
            guard subscription.subscriptionPeriod.unit == .month,
                  subscription.subscriptionPeriod.value == 1 else {
                self.product = nil
                EventQueue.shared.push(["kind": "error", "message": "Supporter product must be a monthly subscription."])
                return
            }
            EventQueue.shared.push(["kind": "products", "price_label": product.displayPrice + " / month"])
            for await result in Transaction.unfinished { accept(result) }
        } catch {
            EventQueue.shared.push(["kind": "error", "message": "Cannot load App Store pricing. Try again."])
        }
    }

    func purchase(token: String) async {
        guard !purchasing, let product, let account = UUID(uuidString: token) else {
            EventQueue.shared.push(["kind": "error", "message": "Connect your account and load App Store pricing first."])
            return
        }
        purchasing = true
        defer { purchasing = false }
        do {
            switch try await product.purchase(options: [.appAccountToken(account)]) {
            case .success(let result):
                guard case .verified = result else {
                    EventQueue.shared.push(["kind": "error", "message": "Apple could not verify the purchase."])
                    return
                }
                accept(result)
            case .pending:
                EventQueue.shared.push(["kind": "pending", "message": "Purchase awaits approval. Apple will notify the game when ready."])
            case .userCancelled:
                EventQueue.shared.push(["kind": "cancelled", "message": "Purchase cancelled."])
            @unknown default:
                EventQueue.shared.push(["kind": "error", "message": "Unexpected App Store result. Use Restore purchases before trying again."])
            }
        } catch {
            EventQueue.shared.push(["kind": "error", "message": "App Store purchase failed. Use Restore purchases if you were charged."])
        }
    }

    func restore() async {
        do {
            try await AppStore.sync()
            var count = 0
            for await result in Transaction.currentEntitlements {
                if case .verified(let transaction) = result, transaction.productID == productID {
                    accept(result); count += 1
                }
            }
            EventQueue.shared.push(["kind": "restored", "message": count == 0 ? "No active Supporter purchase found for this Apple ID." : "Purchases found. Confirming with the game server."])
        } catch {
            EventQueue.shared.push(["kind": "error", "message": "Could not restore purchases. Try again."])
        }
    }

    func finish(_ id: String) async {
        guard let transaction = unfinished[id] else { return }
        await transaction.finish()
        unfinished.removeValue(forKey: id)
    }

    func request(_ command: [String: String]) async {
        switch command["action"] {
        case "configure":
            if let id = command["product_id"], !id.isEmpty { await configure(id) }
        case "purchase":
            if let token = command["app_account_token"] { await purchase(token: token) }
        case "restore": await restore()
        case "finish":
            if let id = command["transaction_id"] { await finish(id) }
        default: break
        }
    }
}

@_cdecl("omoba_storekit_request")
public func omobaStoreKitRequest(_ value: UnsafePointer<CChar>?) {
    guard let value else { return }
    let length = strnlen(value, 4097)
    guard length <= 4096,
          let data = String(cString: value).data(using: .utf8),
          let command = (try? JSONSerialization.jsonObject(with: data)) as? [String: String] else { return }
    Task { @MainActor in await SupporterStore.shared.request(command) }
}

@_cdecl("omoba_storekit_poll")
public func omobaStoreKitPoll(_ buffer: UnsafeMutablePointer<CChar>?, _ capacity: Int32) -> Int32 {
    guard let buffer, capacity > 0 else { return 0 }
    return EventQueue.shared.copyNext(buffer, capacity: Int(capacity))
}
