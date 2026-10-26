import Foundation
import UserNotifications

@_silgen_name("__pushups_probe_install")
private func pushupsProbeInstall()

@_silgen_name("__pushups_probe_rotate")
private func pushupsProbeRotate() -> Int32

@_silgen_name("__pushups_probe_seal")
private func pushupsProbeSeal(
    _ plaintext: UnsafePointer<UInt8>,
    _ length: Int,
    _ accessGroup: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("__pushups_probe_free")
private func pushupsProbeFree(_ sealed: UnsafeMutablePointer<CChar>?)

final class Settlement: @unchecked Sendable {
    private let lock = NSLock()
    private var calls = 0
    private var firstContent: UNNotificationContent?
    private var firstTime: DispatchTime?

    func record(_ content: UNNotificationContent) {
        lock.lock()
        calls += 1
        if firstContent == nil {
            firstContent = content
            firstTime = DispatchTime.now()
        }
        lock.unlock()
    }

    var snapshot: (count: Int, content: UNNotificationContent?, time: DispatchTime?) {
        lock.lock()
        defer { lock.unlock() }
        return (calls, firstContent, firstTime)
    }
}

var failures: [String] = []

func expect(_ condition: Bool, _ label: String) {
    print("\(condition ? "ok" : "FAIL") \(label)")
    if !condition {
        failures.append(label)
    }
}

func request(_ payload: [String: Any]) -> UNNotificationRequest {
    let content = UNMutableNotificationContent()
    content.title = "Safe title"
    content.body = "Safe body"
    content.subtitle = "Kept subtitle"
    content.categoryIdentifier = "kept-category"
    content.threadIdentifier = "original-thread"
    content.badge = 7
    content.sound = .default
    content.userInfo = payload
    return UNNotificationRequest(identifier: "reused-apns-id", content: content, trigger: nil)
}

@discardableResult
func wait(_ settlement: Settlement, seconds: Double = 5) -> Bool {
    let deadline = DispatchTime.now() + seconds
    while settlement.snapshot.count == 0, DispatchTime.now() < deadline {
        Thread.sleep(forTimeInterval: 0.001)
    }
    expect(settlement.snapshot.count == 1, "one completion within \(seconds) seconds")
    return settlement.snapshot.count == 1
}

func receive(_ service: PushupsNotificationService, _ payload: [String: Any]) -> Settlement {
    let result = Settlement()
    service.didReceive(request(payload), withContentHandler: result.record)
    return result
}

func outcome(_ result: Settlement) -> String? {
    result.snapshot.content?.userInfo["pushups.rendering-outcome"] as? String
}

let service = PushupsNotificationService()
let rendered = receive(service, ["kind": "render", "original": "exact 🦀 e\u{301}\n", "unrelated": 42])
wait(rendered)
expect(outcome(rendered) == "rendered", "successful transformation")
if let content = rendered.snapshot.content {
    expect(content.title == "Rendered title" && content.body == "Decrypted body", "rendered alert fields")
    expect(content.threadIdentifier == "thread-1", "rendered thread")
    expect(content.userInfo["pushups.notification-data"] as? String == "exact 🦀 e\u{301}\n", "exact Unicode tap payload")
    expect(content.userInfo["pushups.notification-navigate"] as? String == "https://example.com", "navigation mapping")
    expect(content.userInfo["unrelated"] as? Int == 42, "unrelated payload preserved")
    expect(content.badge == 7 && content.categoryIdentifier == "kept-category", "original presentation metadata preserved")
    expect(content.subtitle == "Kept subtitle" && content.sound == UNNotificationSound.default, "original subtitle and sound preserved")
}

// The probe installs the crate, rotates the key in the protected keychain and seals a push the way a sender would.
pushupsProbeInstall()
expect(pushupsProbeRotate() == 0, "key rotation registers")
guard let accessGroup = Bundle.main.object(forInfoDictionaryKey: "PushupsKeychainAccessGroup") as? String else {
    failures.append("the harness Info.plist declares no keychain access group")
    print("\(failures.count) failures")
    exit(1)
}
let sealedPlaintext = Data(#"{"kind":"render","original":"sealed-plaintext"}"#.utf8)
let sealedPointer: UnsafeMutablePointer<CChar>? = sealedPlaintext.withUnsafeBytes { bytes in
    accessGroup.withCString { group in
        pushupsProbeSeal(bytes.bindMemory(to: UInt8.self).baseAddress!, bytes.count, group)
    }
}
expect(sealedPointer != nil, "the sender sealed the plaintext")
if let sealedPointer {
    let sealedText = String(cString: sealedPointer)
    pushupsProbeFree(sealedPointer)
    let opened = receive(service, ["pushups": sealedText, "original": "ciphertext"])
    wait(opened)
    expect(outcome(opened) == "rendered", "the encrypted push renders the plaintext")
    expect(opened.snapshot.content?.title == "Rendered title" && opened.snapshot.content?.body == "Decrypted body", "the rendered alert is the plaintext's")
    expect(opened.snapshot.content?.userInfo["pushups.notification-data"] as? String == "sealed-plaintext", "the decrypted tap payload")
    var corrupted = Array(sealedText)
    corrupted[0] = corrupted[0] == "A" ? "B" : "A"
    let corruptedPush = receive(service, ["pushups": String(corrupted), "original": "ciphertext"])
    wait(corruptedPush)
    expect(outcome(corruptedPush) == "failed", "the corrupted envelope settles failed")
    expect(corruptedPush.snapshot.content?.title == "Safe title" && corruptedPush.snapshot.content?.body == "Safe body", "the corrupted envelope keeps the safe alert")
    expect(corruptedPush.snapshot.content?.userInfo["original"] as? String == "ciphertext", "the corrupted envelope keeps the sender payload")
}

let cleared = receive(service, ["kind": "clear"])
wait(cleared)
if let content = cleared.snapshot.content {
    expect(content.title == "" && content.body == "", "empty title and absent body clear alert fields")
    expect(content.threadIdentifier == "", "empty tag clears thread")
    expect(content.userInfo["pushups.notification-data"] as? String == "", "empty tap payload stays present")
    expect(content.userInfo["pushups.notification-navigate"] as? String == "", "empty navigation stays present")
}

let minimal = receive(service, ["kind": "minimal", "unrelated": "kept"])
wait(minimal)
expect(minimal.snapshot.content?.threadIdentifier == "original-thread", "absent tag preserves thread")
expect(minimal.snapshot.content?.userInfo["unrelated"] as? String == "kept", "absent data preserves original payload")
expect(minimal.snapshot.content?.userInfo["pushups.notification-data"] == nil, "absent data remains absent")

for (kind, marker) in [("error", "failed"), ("panic", "panicked"), ("icon", "failed")] {
    let fallback = receive(service, ["kind": kind, "original": "ciphertext"])
    wait(fallback)
    expect(outcome(fallback) == marker, "\(kind) outcome")
    expect(fallback.snapshot.content?.title == "Safe title", "\(kind) preserves safe title")
    expect(fallback.snapshot.content?.body == "Safe body", "\(kind) preserves safe body")
    expect(fallback.snapshot.content?.userInfo["original"] as? String == "ciphertext", "\(kind) preserves sender payload")
}

// The harness embeds an Info.plist declaring `PushupsNotificationFiltering`, which the default initializer reads.
let suppressed = receive(service, ["kind": "no-alert", "original": "ciphertext"])
wait(suppressed)
if let content = suppressed.snapshot.content {
    expect(content.title.isEmpty && content.subtitle.isEmpty && content.body.isEmpty, "declared no-alert clears every alert text")
    expect(content.sound == nil && content.attachments.isEmpty, "declared no-alert has no sound or attachments")
    expect(content.userInfo.isEmpty, "declared no-alert completes with fresh content, without the original payload or a marker")
}

let undeclaredService = PushupsNotificationService(filteringDeclared: false)
let undeclared = receive(undeclaredService, ["kind": "no-alert", "original": "ciphertext"])
wait(undeclared)
expect(outcome(undeclared) == "suppression-undeclared", "undeclared no-alert outcome")
expect(undeclared.snapshot.content?.title == "Safe title" && undeclared.snapshot.content?.body == "Safe body", "undeclared no-alert keeps the safe alert")
expect(undeclared.snapshot.content?.userInfo["original"] as? String == "ciphertext", "undeclared no-alert keeps the sender payload")

let expired = receive(service, ["sleep": 300])
service.serviceExtensionTimeWillExpire()
service.serviceExtensionTimeWillExpire()
wait(expired)
expect(outcome(expired) == "expired", "platform expiration")
expect(expired.snapshot.content?.body == "Safe body", "expiration preserves safe alert")

let superseded = receive(service, ["sleep": 300])
let replacement = receive(service, ["original": "replacement"])
wait(superseded)
wait(replacement)
expect(outcome(superseded) == "superseded", "same-instance supersession")
expect(replacement.snapshot.content?.userInfo["pushups.notification-data"] as? String == "replacement", "replacement content")

let otherService = PushupsNotificationService()
let independent = receive(otherService, ["sleep": 300, "original": "independent"])
let concurrent = receive(service, ["original": "concurrent"])
wait(independent)
wait(concurrent)
expect(outcome(independent) == "rendered" && outcome(concurrent) == "rendered", "separate service instances remain independent")

var releasingService: PushupsNotificationService? = PushupsNotificationService()
let released = receive(releasingService!, ["sleep": 300])
releasingService = nil
wait(released)
expect(outcome(released) == "superseded", "deallocation settles pending request")

Thread.sleep(forTimeInterval: 0.4)
expect(expired.snapshot.count == 1 && superseded.snapshot.count == 1 && released.snapshot.count == 1, "late workers cannot complete twice")

let started = DispatchTime.now()
let bounded = receive(service, ["sleep": 26_000])
wait(bounded, seconds: 26)
expect(outcome(bounded) == "expired", "watchdog expiration without platform callback")
expect(bounded.snapshot.time.map { $0 <= started + 26 } ?? false, "completion remains inside platform budget")
Thread.sleep(forTimeInterval: 1.5)
expect(bounded.snapshot.count == 1, "watchdog rejects late result")

if !failures.isEmpty {
    print("\(failures.count) failures")
    exit(1)
}
