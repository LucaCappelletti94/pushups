import Foundation
import UserNotifications

private typealias PushupsFinish = @convention(c) (
    UnsafeMutableRawPointer?, Int32,
    UnsafePointer<UInt8>?, Int,
    UnsafePointer<UInt8>?, Int,
    UnsafePointer<UInt8>?, Int,
    UnsafePointer<UInt8>?, Int,
    UnsafePointer<UInt8>?, Int,
    UnsafePointer<UInt8>?, Int
) -> Void

private final class PushupsCompletion {
    let content: UNMutableNotificationContent
    let handler: (UNNotificationContent) -> Void

    init(
        content: UNMutableNotificationContent,
        handler: @escaping (UNNotificationContent) -> Void
    ) {
        self.content = content
        self.handler = handler
    }

    func finish(
        outcome: Int32,
        marker: String,
        title: String?,
        body: String?,
        tag: String?,
        data: String?,
        navigate: String?
    ) {
        // Apple's filtering contract is a fresh content with no alert fields, so no marker rides on it.
        if outcome == 6 {
            handler(UNNotificationContent())
            return
        }
        if outcome == 0, let title {
            content.title = title
            content.body = body ?? ""
            if let tag {
                content.threadIdentifier = tag
            }
            if let data {
                content.userInfo["pushups.notification-data"] = data
            }
            if let navigate {
                content.userInfo["pushups.notification-navigate"] = navigate
            }
        }
        content.userInfo["pushups.rendering-outcome"] = marker
        handler(content)
    }
}

private func text(_ pointer: UnsafePointer<UInt8>?, _ length: Int) -> String? {
    guard let pointer else { return nil }
    return String(decoding: UnsafeBufferPointer(start: pointer, count: length), as: UTF8.self)
}

private func pushupsFinish(
    _ token: UnsafeMutableRawPointer?,
    _ outcome: Int32,
    _ marker: UnsafePointer<UInt8>?,
    _ markerLength: Int,
    _ title: UnsafePointer<UInt8>?,
    _ titleLength: Int,
    _ body: UnsafePointer<UInt8>?,
    _ bodyLength: Int,
    _ tag: UnsafePointer<UInt8>?,
    _ tagLength: Int,
    _ data: UnsafePointer<UInt8>?,
    _ dataLength: Int,
    _ navigate: UnsafePointer<UInt8>?,
    _ navigateLength: Int
) {
    // Rust transfers this retained token to exactly one terminal callback.
    let completion = Unmanaged<PushupsCompletion>.fromOpaque(token!).takeRetainedValue()
    completion.finish(
        outcome: outcome,
        marker: text(marker, markerLength)!,
        title: text(title, titleLength),
        body: text(body, bodyLength),
        tag: text(tag, tagLength),
        data: text(data, dataLength),
        navigate: text(navigate, navigateLength)
    )
}

@_silgen_name("__pushups_notification_service_create")
private func pushupsCreate(_ filteringDeclared: Bool) -> UnsafeMutableRawPointer

@_silgen_name("__pushups_notification_service_receive")
private func pushupsReceive(
    _ service: UnsafeMutableRawPointer,
    _ userInfo: UnsafePointer<UInt8>,
    _ userInfoLength: Int,
    _ budgetNanoseconds: UInt64,
    _ finish: PushupsFinish,
    _ token: UnsafeMutableRawPointer
)

@_silgen_name("__pushups_notification_service_expired")
private func pushupsExpired(_ service: UnsafeMutableRawPointer)

@_silgen_name("__pushups_notification_service_deallocated")
private func pushupsDeallocated(_ service: UnsafeMutableRawPointer)

/// Transforms an alert through the Rust `rendering_handler` callback.
public final class PushupsNotificationService: UNNotificationServiceExtension {
    private let service: UnsafeMutableRawPointer

    /// Reads the extension's `PushupsNotificationFiltering` declaration from its `Info.plist`.
    public override convenience init() {
        self.init(filteringDeclared: Bundle.main.object(forInfoDictionaryKey: "PushupsNotificationFiltering") as? Bool ?? false)
    }

    init(filteringDeclared: Bool) {
        service = pushupsCreate(filteringDeclared)
        super.init()
    }

    public override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler handler: @escaping (UNNotificationContent) -> Void
    ) {
        let started = DispatchTime.now().uptimeNanoseconds
        let limit: UInt64 = 25_000_000_000
        let content = request.content.mutableCopy() as! UNMutableNotificationContent
        guard let json = try? JSONSerialization.data(withJSONObject: request.content.userInfo) else {
            let elapsed = DispatchTime.now().uptimeNanoseconds - started
            content.userInfo["pushups.rendering-outcome"] = elapsed >= limit ? "expired" : "failed"
            handler(content)
            return
        }
        let completion = PushupsCompletion(content: content, handler: handler)
        json.withUnsafeBytes { buffer in
            let pointer = buffer.baseAddress!.assumingMemoryBound(to: UInt8.self)
            let token = Unmanaged.passRetained(completion).toOpaque()
            let elapsed = DispatchTime.now().uptimeNanoseconds - started
            let remaining = elapsed < limit ? limit - elapsed : 0
            pushupsReceive(service, pointer, buffer.count, remaining, pushupsFinish, token)
        }
    }

    public override func serviceExtensionTimeWillExpire() {
        pushupsExpired(service)
    }

    deinit {
        pushupsDeallocated(service)
    }
}
