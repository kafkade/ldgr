#if os(iOS)
import Foundation
import WatchConnectivity
import LdgrSwift

/// Sends pre-computed financial summaries to the paired Apple Watch.
///
/// All delegate callbacks arrive off-main; `nonisolated` methods hop
/// back to the main actor before mutating observable state.
@MainActor
@Observable
final class WatchConnectivityManager: NSObject, @preconcurrency WCSessionDelegate {
    private(set) var isReachable = false
    private var lastSummary: WatchSummary?

    override init() {
        super.init()
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    // MARK: - Summary Computation

    func sendUpdate(from store: VaultDataStore, client: LdgrClient) async {
        let summary = await computeSummary(from: store, client: client)
        send(summary)
    }

    private func computeSummary(
        from store: VaultDataStore,
        client: LdgrClient
    ) async -> WatchSummary {
        let netWorth = store.netWorthByCommodity.map {
            WatchSummary.NetWorthEntry(
                commodity: $0.commodity,
                amount: formatDecimal($0.amount)
            )
        }

        let portfolio = derivePortfolio(from: store)
        let budget = await computeBudget(client: client, store: store)
        let dailySpend = computeDailySpend(from: store)

        return WatchSummary(
            netWorth: netWorth,
            portfolio: portfolio,
            budget: budget,
            dailySpend: dailySpend,
            watchlist: [],
            lastUpdated: Date()
        )
    }

    private func derivePortfolio(from store: VaultDataStore) -> [WatchSummary.PortfolioHolding] {
        let types = store.accountTypesByName
        return store.balances.compactMap { entry -> WatchSummary.PortfolioHolding? in
            guard types[entry.account] == .asset else { return nil }
            guard let amount = Decimal(string: entry.amount), amount != 0 else { return nil }
            return WatchSummary.PortfolioHolding(
                commodity: entry.commodity,
                amount: entry.amount
            )
        }
    }

    private func computeBudget(
        client: LdgrClient,
        store: VaultDataStore
    ) async -> WatchSummary.BudgetSummary? {
        let calendar = Calendar.current
        let now = Date()
        let components = calendar.dateComponents([.year, .month], from: now)
        guard let startOfMonth = calendar.date(from: components) else { return nil }

        var next = DateComponents()
        next.month = 1
        guard let startOfNext = calendar.date(byAdding: next, to: startOfMonth) else { return nil }

        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd"
        let begin = fmt.string(from: startOfMonth)
        let end = fmt.string(from: startOfNext)

        let monthFmt = DateFormatter()
        monthFmt.dateFormat = "MMMM yyyy"
        let monthLabel = monthFmt.string(from: now)

        let monthlyBalances: [BalanceEntry]
        do {
            monthlyBalances = try await Task.detached {
                try client.balance(accountFilter: nil, beginDate: begin, endDate: end)
            }.value
        } catch {
            return nil
        }

        let types = store.accountTypesByName
        let expenses = monthlyBalances.filter { types[$0.account] == .expense }

        let total = expenses.reduce(Decimal.zero) { sum, entry in
            sum + (Decimal(string: entry.amount) ?? 0)
        }

        let categories = expenses
            .sorted { abs(Decimal(string: $0.amount) ?? 0) > abs(Decimal(string: $1.amount) ?? 0) }
            .prefix(5)
            .map { entry in
                let shortName = shortAccountName(entry.account)
                return WatchSummary.BudgetCategory(name: shortName, amount: entry.amount)
            }

        return WatchSummary.BudgetSummary(
            monthLabel: monthLabel,
            totalSpent: formatDecimal(total),
            categories: Array(categories)
        )
    }

    private func computeDailySpend(from store: VaultDataStore) -> String? {
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd"
        let today = fmt.string(from: Date())

        let total = store.transactions
            .filter { $0.date == today }
            .flatMap(\.postings)
            .filter { posting in
                guard let account = store.account(byId: posting.accountId) else { return false }
                return account.type == .expense
            }
            .reduce(Decimal.zero) { sum, posting in
                sum + (Decimal(string: posting.amount ?? "0") ?? 0)
            }

        guard total != 0 else { return nil }
        return formatDecimal(total)
    }

    // MARK: - Sending

    private func send(_ summary: WatchSummary) {
        lastSummary = summary
        guard WCSession.isSupported(),
              WCSession.default.activationState == .activated,
              WCSession.default.isPaired,
              WCSession.default.isWatchAppInstalled else { return }

        guard let data = try? JSONEncoder().encode(summary) else { return }
        try? WCSession.default.updateApplicationContext(["summary": data])
    }

    // MARK: - Helpers

    private func formatDecimal(_ value: Decimal) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        return formatter.string(from: value as NSDecimalNumber) ?? "\(value)"
    }

    private func shortAccountName(_ name: String) -> String {
        let parts = name.split(separator: ":")
        return parts.count > 1
            ? String(parts.dropFirst().joined(separator: ":"))
            : name
    }

    // MARK: - WCSessionDelegate

    nonisolated func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: Error?
    ) {
        let reachable = session.isReachable
        Task { @MainActor in
            self.isReachable = reachable
        }
    }

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        session.activate()
    }

    nonisolated func session(
        _ session: WCSession,
        didReceiveMessage message: [String: Any],
        replyHandler: @escaping ([String: Any]) -> Void
    ) {
        // `replyHandler` is not `Sendable`, so wrap it to carry it into the
        // main-actor task without a data-race diagnostic. Calling it is safe.
        let reply = UncheckedSendable(replyHandler)
        Task { @MainActor in
            if let summary = self.lastSummary,
               let data = try? JSONEncoder().encode(summary) {
                reply.value(["summary": data])
            } else {
                reply.value([:])
            }
        }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        let reachable = session.isReachable
        Task { @MainActor in
            self.isReachable = reachable
        }
    }
}

#else
import Foundation
import LdgrSwift

/// macOS stub — WatchConnectivity is unavailable on macOS, so paired-watch
/// updates are a no-op. Mirrors the iOS interface used by the shared views.
@MainActor
@Observable
final class WatchConnectivityManager {
    private(set) var isReachable = false

    init() {}

    func sendUpdate(from store: VaultDataStore, client: LdgrClient) async {}
}
#endif
