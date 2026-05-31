import Foundation
import WatchConnectivity
import WidgetKit

/// Receives pre-computed financial summaries from the iPhone companion.
///
/// Caches the latest summary in the shared App Group `UserDefaults`
/// so the WidgetKit complication extension can read it independently.
@MainActor
@Observable
final class PhoneConnectivityManager: NSObject, @preconcurrency WCSessionDelegate {
    private(set) var summary: WatchSummary?
    private(set) var isReachable = false

    override init() {
        super.init()
        loadCachedSummary()
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    /// Ask the iPhone for fresh data (only works when reachable).
    func requestUpdate() {
        guard WCSession.default.isReachable else { return }
        WCSession.default.sendMessage(["request": "summary"]) { [weak self] reply in
            guard let data = reply["summary"] as? Data,
                  let decoded = try? JSONDecoder().decode(WatchSummary.self, from: data) else { return }
            Task { @MainActor in
                self?.save(decoded)
            }
        } errorHandler: { _ in }
    }

    // MARK: - Persistence

    private func loadCachedSummary() {
        guard let defaults = UserDefaults(suiteName: WatchSummary.appGroupId),
              let data = defaults.data(forKey: WatchSummary.defaultsKey) else { return }
        summary = try? JSONDecoder().decode(WatchSummary.self, from: data)
    }

    private func save(_ newSummary: WatchSummary) {
        summary = newSummary
        if let defaults = UserDefaults(suiteName: WatchSummary.appGroupId),
           let data = try? JSONEncoder().encode(newSummary) {
            defaults.set(data, forKey: WatchSummary.defaultsKey)
        }
        WidgetCenter.shared.reloadAllTimelines()
    }

    // MARK: - WCSessionDelegate

    nonisolated func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: Error?
    ) {
        Task { @MainActor in
            self.isReachable = session.isReachable
            if let data = session.receivedApplicationContext["summary"] as? Data,
               let decoded = try? JSONDecoder().decode(WatchSummary.self, from: data) {
                self.save(decoded)
            }
        }
    }

    nonisolated func session(
        _ session: WCSession,
        didReceiveApplicationContext applicationContext: [String: Any]
    ) {
        Task { @MainActor in
            if let data = applicationContext["summary"] as? Data,
               let decoded = try? JSONDecoder().decode(WatchSummary.self, from: data) {
                self.save(decoded)
            }
        }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        Task { @MainActor in
            self.isReachable = session.isReachable
        }
    }
}
