import Foundation
import WidgetKit
import LdgrSwift

/// Writes pre-computed financial summaries to the iOS App Group so
/// the home screen widget extension can read them without vault access.
///
/// Clears the cache when the vault locks to prevent stale financial
/// data from being visible on the lock screen.
@MainActor
final class WidgetDataManager {
    private let appGroupId = WatchSummary.iosAppGroupId
    private let dataKey = WatchSummary.iosDefaultsKey
    private let unlockedKey = WatchSummary.iosUnlockedKey

    func sendUpdate(from store: VaultDataStore, client: LdgrClient) async {
        let summary = await computeSummary(from: store, client: client)
        save(summary)
    }

    /// Mark vault as locked and clear cached financial data.
    func clearOnLock() {
        guard let defaults = UserDefaults(suiteName: appGroupId) else { return }
        defaults.removeObject(forKey: dataKey)
        defaults.set(false, forKey: unlockedKey)
        WidgetCenter.shared.reloadAllTimelines()
    }

    // MARK: - Computation

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

    private func derivePortfolio(
        from store: VaultDataStore
    ) -> [WatchSummary.PortfolioHolding] {
        let types = store.accountTypesByName
        return store.balances.compactMap { entry in
            guard types[entry.account] == .asset else { return nil }
            guard let amount = Decimal(string: entry.amount),
                  amount != 0 else { return nil }
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
        guard let startOfMonth = calendar.date(from: components) else {
            return nil
        }

        var next = DateComponents()
        next.month = 1
        guard let startOfNext = calendar.date(
            byAdding: next, to: startOfMonth
        ) else { return nil }

        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd"

        let monthFmt = DateFormatter()
        monthFmt.dateFormat = "MMMM yyyy"
        let monthLabel = monthFmt.string(from: now)

        let monthlyBalances: [BalanceEntry]
        do {
            monthlyBalances = try await Task.detached {
                try client.balance(
                    accountFilter: nil,
                    beginDate: fmt.string(from: startOfMonth),
                    endDate: fmt.string(from: startOfNext)
                )
            }.value
        } catch {
            return nil
        }

        let types = store.accountTypesByName
        let expenses = monthlyBalances.filter {
            types[$0.account] == .expense
        }

        let total = expenses.reduce(Decimal.zero) { sum, entry in
            sum + (Decimal(string: entry.amount) ?? 0)
        }

        let categories = expenses
            .sorted {
                abs(Decimal(string: $0.amount) ?? 0)
                    > abs(Decimal(string: $1.amount) ?? 0)
            }
            .prefix(5)
            .map { entry in
                WatchSummary.BudgetCategory(
                    name: shortAccountName(entry.account),
                    amount: entry.amount
                )
            }

        return WatchSummary.BudgetSummary(
            monthLabel: monthLabel,
            totalSpent: formatDecimal(total),
            categories: Array(categories)
        )
    }

    private func computeDailySpend(
        from store: VaultDataStore
    ) -> String? {
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd"
        let today = fmt.string(from: Date())

        let total = store.transactions
            .filter { $0.date == today }
            .flatMap(\.postings)
            .filter { posting in
                guard let account = store.account(
                    byId: posting.accountId
                ) else { return false }
                return account.type == .expense
            }
            .reduce(Decimal.zero) { sum, posting in
                sum + (Decimal(string: posting.amount ?? "0") ?? 0)
            }

        guard total != 0 else { return nil }
        return formatDecimal(total)
    }

    // MARK: - Persistence

    private func save(_ summary: WatchSummary) {
        guard let defaults = UserDefaults(suiteName: appGroupId),
              let data = try? JSONEncoder().encode(summary) else { return }
        defaults.set(data, forKey: dataKey)
        defaults.set(true, forKey: unlockedKey)
        WidgetCenter.shared.reloadAllTimelines()
    }

    // MARK: - Helpers

    private func formatDecimal(_ value: Decimal) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        return formatter.string(
            from: value as NSDecimalNumber
        ) ?? "\(value)"
    }

    private func shortAccountName(_ name: String) -> String {
        let parts = name.split(separator: ":")
        return parts.count > 1
            ? String(parts.dropFirst().joined(separator: ":"))
            : name
    }
}
