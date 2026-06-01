import WidgetKit
import SwiftUI

@main
struct LdgrWidgetBundle: WidgetBundle {
    var body: some Widget {
        NetWorthWidget()
        SpendingWidget()
        PortfolioWidget()
    }
}

// MARK: - Shared Helpers

func loadWidgetData() -> WatchSummary? {
    guard let defaults = UserDefaults(
        suiteName: WatchSummary.iosAppGroupId
    ) else { return nil }

    let unlocked = defaults.bool(forKey: WatchSummary.iosUnlockedKey)
    guard unlocked else { return nil }

    guard let data = defaults.data(
        forKey: WatchSummary.iosDefaultsKey
    ) else { return nil }

    return try? JSONDecoder().decode(WatchSummary.self, from: data)
}

func nextRefreshDate() -> Date {
    Calendar.current.date(
        byAdding: .minute, value: 30, to: Date()
    ) ?? Date()
}

// MARK: - Net Worth Widget

struct NetWorthEntry: TimelineEntry {
    let date: Date
    let netWorth: [WatchSummary.NetWorthEntry]
    let isLocked: Bool
}

struct NetWorthProvider: TimelineProvider {
    func placeholder(in context: Context) -> NetWorthEntry {
        NetWorthEntry(date: .now, netWorth: [], isLocked: false)
    }

    func getSnapshot(
        in context: Context,
        completion: @escaping (NetWorthEntry) -> Void
    ) {
        let data = loadWidgetData()
        completion(NetWorthEntry(
            date: .now,
            netWorth: data?.netWorth ?? [],
            isLocked: data == nil
        ))
    }

    func getTimeline(
        in context: Context,
        completion: @escaping (Timeline<NetWorthEntry>) -> Void
    ) {
        let data = loadWidgetData()
        let entry = NetWorthEntry(
            date: .now,
            netWorth: data?.netWorth ?? [],
            isLocked: data == nil
        )
        completion(Timeline(
            entries: [entry],
            policy: .after(nextRefreshDate())
        ))
    }
}

struct NetWorthWidget: Widget {
    let kind = "NetWorthWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: kind,
            provider: NetWorthProvider()
        ) { entry in
            NetWorthWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Net Worth")
        .description("Your current net worth at a glance.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

// MARK: - Spending Widget

struct SpendingEntry: TimelineEntry {
    let date: Date
    let budget: WatchSummary.BudgetSummary?
    let dailySpend: String?
    let isLocked: Bool
}

struct SpendingProvider: TimelineProvider {
    func placeholder(in context: Context) -> SpendingEntry {
        SpendingEntry(
            date: .now, budget: nil, dailySpend: nil, isLocked: false
        )
    }

    func getSnapshot(
        in context: Context,
        completion: @escaping (SpendingEntry) -> Void
    ) {
        let data = loadWidgetData()
        completion(SpendingEntry(
            date: .now,
            budget: data?.budget,
            dailySpend: data?.dailySpend,
            isLocked: data == nil
        ))
    }

    func getTimeline(
        in context: Context,
        completion: @escaping (Timeline<SpendingEntry>) -> Void
    ) {
        let data = loadWidgetData()
        let entry = SpendingEntry(
            date: .now,
            budget: data?.budget,
            dailySpend: data?.dailySpend,
            isLocked: data == nil
        )
        completion(Timeline(
            entries: [entry],
            policy: .after(nextRefreshDate())
        ))
    }
}

struct SpendingWidget: Widget {
    let kind = "SpendingWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: kind,
            provider: SpendingProvider()
        ) { entry in
            BudgetWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Monthly Spending")
        .description("This month's expense breakdown.")
        .supportedFamilies([.systemMedium])
    }
}

// MARK: - Portfolio Widget

struct PortfolioEntry: TimelineEntry {
    let date: Date
    let holdings: [WatchSummary.PortfolioHolding]
    let isLocked: Bool
}

struct PortfolioProvider: TimelineProvider {
    func placeholder(in context: Context) -> PortfolioEntry {
        PortfolioEntry(date: .now, holdings: [], isLocked: false)
    }

    func getSnapshot(
        in context: Context,
        completion: @escaping (PortfolioEntry) -> Void
    ) {
        let data = loadWidgetData()
        completion(PortfolioEntry(
            date: .now,
            holdings: data?.portfolio ?? [],
            isLocked: data == nil
        ))
    }

    func getTimeline(
        in context: Context,
        completion: @escaping (Timeline<PortfolioEntry>) -> Void
    ) {
        let data = loadWidgetData()
        let entry = PortfolioEntry(
            date: .now,
            holdings: data?.portfolio ?? [],
            isLocked: data == nil
        )
        completion(Timeline(
            entries: [entry],
            policy: .after(nextRefreshDate())
        ))
    }
}

struct PortfolioWidget: Widget {
    let kind = "PortfolioWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: kind,
            provider: PortfolioProvider()
        ) { entry in
            PortfolioWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Portfolio")
        .description("Investment portfolio overview.")
        .supportedFamilies([.systemMedium])
    }
}
