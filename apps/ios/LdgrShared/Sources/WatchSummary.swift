import Foundation

/// Pre-computed financial summary sent from iPhone to Apple Watch.
///
/// The Watch never decrypts the vault directly — all data arrives as
/// pre-computed summaries via WatchConnectivity.
struct WatchSummary: Codable, Sendable {
    struct NetWorthEntry: Codable, Sendable {
        let commodity: String
        let amount: String
    }

    struct PortfolioHolding: Codable, Sendable {
        let commodity: String
        let amount: String
    }

    struct BudgetCategory: Codable, Sendable {
        let name: String
        let amount: String
    }

    struct BudgetSummary: Codable, Sendable {
        let monthLabel: String
        let totalSpent: String
        let categories: [BudgetCategory]
    }

    struct WatchlistEntry: Codable, Sendable {
        let symbol: String
        let price: String?
        let change: String?
    }

    let netWorth: [NetWorthEntry]
    let portfolio: [PortfolioHolding]
    let budget: BudgetSummary?
    let dailySpend: String?
    let watchlist: [WatchlistEntry]
    let lastUpdated: Date

    /// App group identifier shared between the watch app and its widget extension.
    static let appGroupId = "group.com.kafkade.ldgr.watch"

    /// UserDefaults key for the cached summary.
    static let defaultsKey = "watchSummary"

    /// App group for iOS app and its home screen widget extension.
    static let iosAppGroupId = "group.com.kafkade.ldgr"

    /// UserDefaults key for the iOS widget data cache.
    static let iosDefaultsKey = "widgetData"

    /// UserDefaults key indicating whether the vault is currently unlocked.
    static let iosUnlockedKey = "vaultUnlocked"
}
