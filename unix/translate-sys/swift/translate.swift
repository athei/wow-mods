// Swift wrapper around Apple's Translation framework, exposed via @_cdecl
// for the Rust side of wow_translate.so.
//
// Translation.framework's public API is SwiftUI-bound: a TranslationSession
// is delivered through the .translationTask View modifier, not constructed
// directly. We stand up one persistent SwiftUI PairView per (src, tgt)
// language pair the caller has ever used, each carrying its own
// TranslationSession.Configuration (frozen at creation, never mutated,
// never invalidated) so the inner closure fires once on PairView mount
// and stays alive — pumping many requests through one reused session via
// an AsyncStream. No per-request Configuration mutation, no invalidate(),
// no shared mutable pendingRequest.
//
// The host window can be torn down out from under us (destroying the
// D3D9 device destroys the window the hosting view sits in). SwiftUI then
// cancels every PairView's .translationTask, terminating its stream.
// submit() detects the dead pump synchronously — yield returns
// .terminated — and restages the pair under a bumped generation, which
// remounts a fresh PairView (a new view identity is guaranteed to fire
// .translationTask) and revives the pump with a new warm session.
//
// On macOS 26.4 and newer each Configuration is built with
// preferredStrategy: .highFidelity to opt into the larger on-device
// translation model. On 15.0–26.3 the strategy-less initializer is
// used and Apple picks the default model. The choice is made by an
// `if #available(macOS 26.4, *)` runtime probe, so the same dylib
// works on both.
//
// The single NSHostingView (containing the DriverHostView root) is
// re-parented into the game's existing NSWindow (the one the D3D9→Metal
// layer's AttachMetalLayer set up via macdrv) so any macOS "Download language
// pack?" sheet attaches to a visible window — the game window itself.
//
// Async→sync bridge via DispatchSemaphore; the @_cdecl call blocks the
// caller (the PE worker thread post-marshal) until the SwiftUI task
// completes, or until a single global deadline elapses.
//
// Status discriminants match wow_shared::TranslateStatus byte for byte:
//   0 = Internal, 1 = Ok, 2 = Unavailable, 3 = InvalidParams,
//   4 = UnsupportedPair, 5 = LanguagePackMissing, 6 = Timeout,
//   7 = BufferTooSmall.
//
// Failure paths log to stderr via `print` so the flow is observable in
// Console.app (which captures the wine process's stderr) even when
// RUST_LOG filters the unix-side `log` crate output. `print` is preferred
// over `NSLog` because macOS routes `NSLog` through the unified logging
// system, which rate-limits and may silently drop low-frequency messages
// from a dylib that's still registering with the logging daemon.

import AppKit
import Foundation
import QuartzCore
import SwiftUI
import Translation

private let STATUS_INTERNAL: UInt32 = 0
private let STATUS_OK: UInt32 = 1
private let STATUS_UNAVAILABLE: UInt32 = 2
private let STATUS_INVALID_PARAMS: UInt32 = 3
private let STATUS_UNSUPPORTED_PAIR: UInt32 = 4
private let STATUS_LANGUAGE_PACK_MISSING: UInt32 = 5
private let STATUS_TIMEOUT: UInt32 = 6
private let STATUS_BUFFER_TOO_SMALL: UInt32 = 7

// One deadline per batch (not per message). Covers warm-session batch
// translations (typically <500 ms for the whole array) and first-call cases
// where the user may be confirming the language-pack download sheet. The worker
// translates at most one batch at a time, so the Lua addon's REQUEST_TIMEOUT
// staleness ceiling only needs to exceed this single deadline.
private let TIMEOUT: TimeInterval = 10.0

// MARK: - Driver

@MainActor
private final class TranslationDriver: ObservableObject {

    static let shared = TranslationDriver()

    struct PairKey: Hashable {
        let src: String
        let tgt: String
    }

    /// One mounted `PairView` lifetime. The generation bumps each time the
    /// pair's pump dies with the host window, giving SwiftUI a fresh view
    /// identity so `.translationTask` fires again.
    struct PairInstance: Hashable {
        let key: PairKey
        let gen: Int
    }

    struct Request: Identifiable {
        let id = UUID()
        let texts: [String]
        let srcLang: Locale.Language
        let tgtLang: Locale.Language
        let enqueuedAt = Date()
        let onResult: ([Result<String, TranslationError>]) -> Void
    }

    enum TranslationError: Error {
        case unavailable
        case unsupportedPair
        case languagePackMissing
        case timeout
        case internalError(String)
    }

    @Published var activePairs: [PairInstance] = []

    private var configurations: [PairKey: TranslationSession.Configuration] = [:]
    private var continuations: [PairKey: AsyncStream<Request>.Continuation] = [:]
    private var pendingStreams: [PairInstance: AsyncStream<Request>] = [:]
    private var hostingView: NSHostingView<DriverHostView>?

    private init() {}

    func submit(_ req: Request) {
        guard ensureHostingViewAttached() else {
            print(
                "wow_translate_sys: submit failed — no host NSWindow"
                + " id=\(req.id) src=\(req.srcLang.maximalIdentifier)"
                + " tgt=\(req.tgtLang.maximalIdentifier) items=\(req.texts.count)"
            )
            req.onResult(req.texts.map { _ in .failure(.unavailable) })
            return
        }
        let key = PairKey(
            src: req.srcLang.maximalIdentifier,
            tgt: req.tgtLang.maximalIdentifier
        )
        if continuations[key] == nil {
            let strategyLabel: String
            if #available(macOS 26.4, *) {
                configurations[key] = TranslationSession.Configuration(
                    source: req.srcLang,
                    target: req.tgtLang,
                    preferredStrategy: .highFidelity
                )
                strategyLabel = "highFidelity"
            } else {
                configurations[key] = TranslationSession.Configuration(
                    source: req.srcLang,
                    target: req.tgtLang
                )
                strategyLabel = "default"
            }
            stage(key: key, gen: 0)
            print(
                "wow_translate_sys: standing up \(strategyLabel) session for pair"
                + " \(key.src)->\(key.tgt)"
            )
        }
        guard let cont = continuations[key] else { return }
        if case .terminated = cont.yield(req) {
            // The pump task died: the host window was destroyed, SwiftUI
            // cancelled the PairView's .translationTask, and that terminated
            // the stream. Restage under a bumped generation so a fresh
            // PairView mounts and fires .translationTask again, and re-yield
            // so this request is not lost — it waits in the new unbounded
            // buffer until the fresh pump starts. (A request that yielded
            // .enqueued just before the cancellation is dropped with the old
            // buffer and times out once; the next submit lands here and
            // recovers.)
            let gen = (activePairs.first(where: { $0.key == key })?.gen ?? -1) + 1
            stage(key: key, gen: gen)
            continuations[key]?.yield(req)
            print(
                "wow_translate_sys: pump dead for pair \(key.src)->\(key.tgt)"
                + " — restaged gen=\(gen) id=\(req.id)"
            )
        }
    }

    /// Create a fresh stream+continuation for `key` under generation `gen`,
    /// staging the stream for the matching `PairView`'s `.translationTask`
    /// fire. Replaces the pair's entry in `activePairs` (dropping any
    /// never-taken staged stream of the prior generation) so repeated
    /// restages keep the state bounded.
    private func stage(key: PairKey, gen: Int) {
        preservingPointer {
            let (stream, cont) = AsyncStream<Request>.makeStream(
                bufferingPolicy: .unbounded
            )
            continuations[key] = cont
            let instance = PairInstance(key: key, gen: gen)
            pendingStreams[instance] = stream
            if let idx = activePairs.firstIndex(where: { $0.key == key }) {
                pendingStreams.removeValue(forKey: activePairs[idx])
                activePairs[idx] = instance
            } else {
                activePairs.append(instance)
            }
        }
    }

    /// Frozen `Configuration` for the given pair, created once on first
    /// `submit` and reused forever — including across regenerated
    /// `PairView`s of the same pair. `PairView` passes this into
    /// `.translationTask(_:_:)`; because the value never mutates, the
    /// modifier fires its action exactly once per `PairView` mount.
    func configuration(for key: PairKey) -> TranslationSession.Configuration? {
        configurations[key]
    }

    /// Called by each `PairView`'s `.translationTask` closure to take
    /// ownership of the stream `stage` pre-staged for exactly that view
    /// instance. One-shot and instance-keyed: a stale-generation view that
    /// re-fires after window churn finds no entry for its own instance and
    /// backs off, so it can never steal a newer generation's stream.
    func takeStream(_ instance: PairInstance) -> AsyncStream<Request>? {
        pendingStreams.removeValue(forKey: instance)
    }

    /// Ensure the `NSHostingView` is in the live `NSWindow` hierarchy.
    /// The game window can be destroyed and recreated by a D3D9 Reset, in
    /// which case our subview's `window` goes nil — detect and re-attach.
    /// Returns false if no suitable host window exists (we can't fabricate
    /// one without re-introducing an orderOut'd hidden window, which
    /// breaks sheet attachment).
    private func ensureHostingViewAttached() -> Bool {
        if let v = hostingView, v.window != nil {
            return true
        }
        let host = NSApplication.shared.mainWindow
            ?? NSApplication.shared.windows.first(where: {
                $0.isVisible && $0.contentView != nil
            })
        guard let host, let contentView = host.contentView else {
            return false
        }
        let view = hostingView ?? NSHostingView(rootView: DriverHostView())
        view.frame = NSRect(x: -10, y: -10, width: 1, height: 1)
        view.removeFromSuperview()
        preservingPointer { contentView.addSubview(view) }
        hostingView = view
        print("wow_translate_sys: hosted in window '\(host.title)'")
        return true
    }

    /// Run `body`, then undo the pointer reset AppKit does in response to it.
    ///
    /// Changing a window's view hierarchy invalidates its structural regions,
    /// and AppKit recomputes which pointer belongs at the mouse location
    /// during that window's next display cycle:
    ///
    ///     -[_NSTrackingAreaAKManager setCursorForMouseLocation:]
    ///     -[_NSTrackingAreaAKManager displayCycleUpdateStructuralRegions]
    ///     NSDisplayCycleFlush → CA::Transaction::commit
    ///
    /// Nothing in a wine window claims a cursor for that location, so the
    /// recomputation lands on the system arrow. Wine never learns: its own
    /// cursor state is unchanged, and it only notifies its Mac driver when the
    /// cursor *handle* changes (`server/queue.c`, `set_cursor`:
    /// `if (prev_cursor != new_cursor)`), so nothing re-applies the game's
    /// cursor. It stays lost until something swaps the bitmap, for example
    /// hovering a UI element. Wine's own re-apply path
    /// (`-[WineApplicationController updateCursor:]` off `handleMouseMove:`)
    /// stays silent for the same reason: it believes the cursor is current.
    ///
    /// The restore rides the completion of the transaction the mutation joins,
    /// which is the transaction whose commit runs the reset, so the ordering
    /// comes from CoreAnimation's commit sequence rather than from a guess
    /// about how long AppKit takes. (`addCommitHandler(_:for:)` would express
    /// that more directly but is not in the public SDK.) An existing
    /// completion block is chained rather than replaced, since the property is
    /// per-thread and shared with whatever else joined this transaction.
    ///
    /// `set()` talks to AppKit only, so wine's state stays untouched and
    /// correct throughout, and the restore is skipped unless the pointer
    /// actually changed, so a cursor the game swapped itself is left alone.
    private func preservingPointer(_ body: () -> Void) {
        let saved = NSCursor.current
        body()
        let chained = CATransaction.completionBlock()
        CATransaction.setCompletionBlock {
            chained?()
            guard NSCursor.current !== saved else { return }
            saved.set()
            print("wow_translate_sys: pointer reset by AppKit, restored")
        }
    }
}

// MARK: - SwiftUI views

private struct DriverHostView: View {
    @ObservedObject private var driver = TranslationDriver.shared

    var body: some View {
        ForEach(driver.activePairs, id: \.self) { instance in
            PairView(instance: instance)
        }
    }
}

private struct PairView: View {
    let instance: TranslationDriver.PairInstance

    private var key: TranslationDriver.PairKey { instance.key }

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)
            .translationTask(
                TranslationDriver.shared.configuration(for: key)
            ) { session in
                guard let stream = await TranslationDriver.shared
                    .takeStream(instance)
                else { return }
                await pump(stream: stream, session: session)
            }
    }

    private func pump(
        stream: AsyncStream<TranslationDriver.Request>,
        session: TranslationSession
    ) async {
        for await req in stream {
            // Late-request cancellation: the C-side caller may have
            // already timed out on its semaphore while this batch sat
            // in the stream behind a slow predecessor. Short-circuit
            // rather than tying up the session translating a result no
            // one will read.
            let waited = Date().timeIntervalSince(req.enqueuedAt)
            if waited > TIMEOUT {
                print(
                    "wow_translate_sys: dropping stale batch"
                    + " id=\(req.id) pair=\(key.src)->\(key.tgt)"
                    + " items=\(req.texts.count)"
                    + " waited=\(String(format: "%.2f", waited))s"
                )
                req.onResult(req.texts.map { _ in .failure(.timeout) })
                continue
            }

            do {
                // One round trip for the whole batch. Tag each request with its
                // index as the client identifier so responses (which may come
                // back out of order) map cleanly back to their output slots.
                let batch = req.texts.enumerated().map { index, text in
                    TranslationSession.Request(
                        sourceText: text,
                        clientIdentifier: String(index)
                    )
                }
                let responses = try await session.translations(from: batch)
                var results = [Result<String, TranslationDriver.TranslationError>](
                    repeating: .failure(.internalError("missing response")),
                    count: req.texts.count
                )
                for resp in responses {
                    if let idStr = resp.clientIdentifier,
                        let idx = Int(idStr), idx >= 0, idx < results.count
                    {
                        results[idx] = .success(resp.targetText)
                    }
                }
                req.onResult(results)
            } catch is CancellationError {
                print(
                    "wow_translate_sys: session cancelled"
                    + " id=\(req.id) pair=\(key.src)->\(key.tgt)"
                )
                req.onResult(req.texts.map { _ in .failure(.timeout) })
                return
            } catch let e as TranslationError {
                // A pair-level failure (e.g. unsupported pairing, missing pack)
                // applies to every item in the batch.
                let mapped = mapAppleError(e, pair: key, id: req.id)
                req.onResult(req.texts.map { _ in .failure(mapped) })
            } catch {
                print(
                    "wow_translate_sys: batch error id=\(req.id)"
                    + " pair=\(key.src)->\(key.tgt)"
                    + " err=\(error) type=\(type(of: error))"
                )
                let desc = String(describing: error)
                req.onResult(req.texts.map { _ in .failure(.internalError(desc)) })
            }
        }
    }
}

// MARK: - Error mapping

private func mapAppleError(
    _ e: TranslationError,
    pair: TranslationDriver.PairKey,
    id: UUID
) -> TranslationDriver.TranslationError {
    switch e {
    case .unsupportedLanguagePairing:
        print(
            "wow_translate_sys: UNSUPPORTED PAIR"
            + " id=\(id) pair=\(pair.src)->\(pair.tgt)"
            + " — this language combination cannot be translated;"
            + " no language pack is available from Apple"
        )
        return .unsupportedPair
    default:
        // Apple's TranslationError surface evolves between OS versions.
        // Anything that smells like "no language pack" gets routed to
        // a single explicit ALL-CAPS log line so it's obvious in
        // Console.app what the user has to do. The raw error stays in
        // the log so we can extend the case list later if a new case
        // shows up in practice.
        let desc = String(describing: e).lowercased()
        let looksLikeMissingPack =
            desc.contains("language")
            && (desc.contains("install")
                || desc.contains("download")
                || desc.contains("missing")
                || desc.contains("notinstalled"))
        if looksLikeMissingPack {
            print(
                "wow_translate_sys: LANGUAGE PACK MISSING"
                + " id=\(id) pair=\(pair.src)->\(pair.tgt)"
                + " — install the pack via Settings > General >"
                + " Language & Region > Translation Languages,"
                + " or accept the download sheet attached to the game"
                + " window. Raw error: \(e)"
            )
            return .languagePackMissing
        }
        print(
            "wow_translate_sys: TranslationError (unrecognised)"
            + " id=\(id) pair=\(pair.src)->\(pair.tgt) err=\(e)"
        )
        return .internalError(String(describing: e))
    }
}

private func mapDriverError(_ e: TranslationDriver.TranslationError) -> UInt32 {
    switch e {
    case .unavailable: return STATUS_UNAVAILABLE
    case .unsupportedPair: return STATUS_UNSUPPORTED_PAIR
    case .languagePackMissing: return STATUS_LANGUAGE_PACK_MISSING
    case .timeout: return STATUS_TIMEOUT
    case .internalError: return STATUS_INTERNAL
    }
}

// MARK: - Helpers

private func bcp47Language(_ ptr: UnsafePointer<CChar>?, _ len: UInt32) -> Locale.Language? {
    guard let ptr = ptr, len > 0 else { return nil }
    let buf = UnsafeBufferPointer(start: ptr, count: Int(len))
    let bytes = buf.map { UInt8(bitPattern: $0) }
    guard let tag = String(bytes: bytes, encoding: .utf8), !tag.isEmpty else {
        return nil
    }
    return Locale.Language(identifier: tag)
}

private func utf8String(_ ptr: UnsafePointer<CChar>?, _ len: UInt32) -> String? {
    guard let ptr = ptr, len > 0 else { return nil }
    let buf = UnsafeBufferPointer(start: ptr, count: Int(len))
    let bytes = buf.map { UInt8(bitPattern: $0) }
    return String(bytes: bytes, encoding: .utf8)
}

// MARK: - C ABI

@_cdecl("wow_translate_sync")
public func wow_translate_sync(
    _ count: UInt32,
    _ srcPtr: UnsafePointer<CChar>?,
    _ srcLen: UInt32,
    _ tgtPtr: UnsafePointer<CChar>?,
    _ tgtLen: UInt32,
    _ inLensPtr: UnsafePointer<UInt32>?,
    _ inBytesPtr: UnsafePointer<CChar>?,
    _ inBytesLen: UInt32,
    _ outBytesPtr: UnsafeMutablePointer<CChar>?,
    _ outSlotBytes: UInt32,
    _ outLensPtr: UnsafeMutablePointer<UInt32>?,
    _ outStatusPtr: UnsafeMutablePointer<UInt32>?
) -> UInt32 {
    let n = Int(count)
    guard n > 0,
        let src = bcp47Language(srcPtr, srcLen),
        let tgt = bcp47Language(tgtPtr, tgtLen),
        let inLensPtr = inLensPtr,
        let inBytesPtr = inBytesPtr,
        let outBytesPtr = outBytesPtr,
        let outLensPtr = outLensPtr,
        let outStatusPtr = outStatusPtr
    else {
        return STATUS_INVALID_PARAMS
    }

    let outSlot = Int(outSlotBytes)
    let inLens = UnsafeBufferPointer(start: inLensPtr, count: n)
    let outLens = UnsafeMutableBufferPointer(start: outLensPtr, count: n)
    let outStatus = UnsafeMutableBufferPointer(start: outStatusPtr, count: n)
    let inBase = UnsafeRawPointer(inBytesPtr)

    // Split the packed input into `n` UTF-8 texts using the per-item lengths.
    // Each slot defaults to INVALID_PARAMS / 0 bytes; empty or non-UTF-8 items
    // are excluded from the batch (Apple throws on "") and keep that default.
    var texts: [String?] = []
    texts.reserveCapacity(n)
    var offset = 0
    for i in 0..<n {
        outLens[i] = 0
        outStatus[i] = STATUS_INVALID_PARAMS
        let len = Int(inLens[i])
        guard len > 0, offset + len <= Int(inBytesLen) else {
            texts.append(nil)
            offset += len
            continue
        }
        let p = inBase.advanced(by: offset).assumingMemoryBound(to: CChar.self)
        let s = utf8String(p, UInt32(len))
        texts.append((s?.isEmpty == false) ? s : nil)
        offset += len
    }

    // Collect the translatable items, remembering each one's original slot.
    var batchTexts: [String] = []
    var batchIndices: [Int] = []
    for (i, t) in texts.enumerated() {
        if let t = t {
            batchTexts.append(t)
            batchIndices.append(i)
        }
    }
    if batchTexts.isEmpty {
        return STATUS_OK
    }

    let semaphore = DispatchSemaphore(value: 0)
    var batchResults: [Result<String, TranslationDriver.TranslationError>]?
    let start = Date()

    let req = TranslationDriver.Request(
        texts: batchTexts,
        srcLang: src,
        tgtLang: tgt,
        onResult: { results in
            batchResults = results
            semaphore.signal()
        }
    )

    DispatchQueue.main.async {
        TranslationDriver.shared.submit(req)
    }

    if semaphore.wait(timeout: .now() + TIMEOUT) == .timedOut {
        print(
            "wow_translate_sys: batch timeout id=\(req.id)"
            + " pair=\(src.maximalIdentifier)->\(tgt.maximalIdentifier)"
            + " items=\(batchTexts.count) waited=\(Int(TIMEOUT))s"
        )
        for i in batchIndices {
            outStatus[i] = STATUS_TIMEOUT
            outLens[i] = 0
        }
        return STATUS_TIMEOUT
    }

    // Read the closure-written results only on the not-timedOut branch — the
    // semaphore signal↔wait establishes the happens-before for this read.
    guard let results = batchResults, results.count == batchTexts.count else {
        for i in batchIndices {
            outStatus[i] = STATUS_INTERNAL
            outLens[i] = 0
        }
        return STATUS_INTERNAL
    }

    let elapsed = Date().timeIntervalSince(start)
    var okCount = 0
    for (b, result) in results.enumerated() {
        let i = batchIndices[b]
        switch result {
        case .success(let s):
            let utf8 = Array(s.utf8)
            if utf8.count > outSlot {
                outStatus[i] = STATUS_BUFFER_TOO_SMALL
                outLens[i] = UInt32(utf8.count)  // required size
            } else {
                let dst = outBytesPtr.advanced(by: i * outSlot)
                utf8.withUnsafeBufferPointer { srcBuf in
                    guard let base = srcBuf.baseAddress else { return }
                    dst.withMemoryRebound(to: UInt8.self, capacity: outSlot) { d in
                        d.update(from: base, count: utf8.count)
                    }
                }
                outStatus[i] = STATUS_OK
                outLens[i] = UInt32(utf8.count)
                okCount += 1
            }
        case .failure(let e):
            outStatus[i] = mapDriverError(e)
            outLens[i] = 0
        }
    }

    print(
        "wow_translate_sys: batch ok=\(okCount)/\(batchTexts.count)"
        + " pair=\(src.maximalIdentifier)->\(tgt.maximalIdentifier)"
        + " ms=\(Int(elapsed * 1000))"
    )
    return STATUS_OK
}
