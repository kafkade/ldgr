import WidgetKit
import SwiftUI

// MARK: - Widget Bundle

@main
struct LdgrWidgets: WidgetBundle {
    var body: some Widget {
        NetWorthWidget()
        DailySpendWidget()
        PortfolioWidget()
    }
}

// MARK: - Shared Helpers

private func loadSummary() -> WatchSummary? {
    guard let defaults = UserDefaults(suiteName: WatchSummary.appGroupId),
          let data = defaults.data(forKey: WatchSummary.defaultsKey) else { return nil }
    return try? JSONDecoder().decode(WatchSummary.self, from: data)
}

private func nextRefresh() -> Date {
    Calendar.current.date(byAdding: .hour, value: 1, to: Date()) ?? Date()
}

// MARK: - Net Worth Widget

struct NetWorthEntry: TimelineEntry {
    let date: Date
    let amount: String?
    let commodity: String?
}

struct NetWorthProvider: TimelineProvider {
    func placeholder(in context: Context) -> NetWorthEntry {
        NetWorthEntry(date: .now, amount: "—", commodity: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (NetWorthEntry) -> Void) {
        let summary = loadSummary()
        let primary = summary?.netWorth.first
        completion(NetWorthEntry(
            date: .now,
            amount: primary?.amount,
            commodity: primary?.commodity
        ))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<NetWorthEntry>) -> Void) {
        let summary = loadSummary()
        let primary = summary?.netWorth.first
        let entry = NetWorthEntry(
            date: .now,
            amount: primary?.amount,
            commodity: primary?.commodity
        )
        completion(Timeline(entries: [entry], policy: .after(nextRefresh())))
    }
}

struct NetWorthWidget: Widget {
    let kind = "NetWorthWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: NetWorthProvider()) { entry in
            NetWorthWidgetView(entry: entry)
        }
        .configurationDisplayName("Net Worth")
        .description("Your current net worth.")
        .supportedFamilies([.accessoryRectangular, .accessoryCircular, .accessoryInline])
    }
}

struct NetWorthWidgetView: View {
    let entry: NetWorthEntry

    @Environment(\.widgetFamily) var family

    var body: some View {
        switch family {
        case .accessoryRectangular:
            VStack(alignment: .leading, spacing: 2) {
                Text("Net Worth")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .widgetAccentable()
                Text(entry.amount ?? "—")
                    .font(.headline.monospacedDigit())
                if let commodity = entry.commodity {
                    Text(commodity)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

        case .accessoryCircular:
            VStack(spacing: 1) {
                Image(systemName: "banknote")
                    .font(.caption)
                    .widgetAccentable()
                Text(entry.amount ?? "—")
                    .font(.caption2.monospacedDigit())
                    .minimumScaleFactor(0.5)
            }

        case .accessoryInline:
            Text("Net Worth: \(entry.amount ?? "—")")

        default:
            Text(entry.amount ?? "—")
        }
    }
}

// MARK: - Daily Spend Widget

struct DailySpendEntry: TimelineEntry {
    let date: Date
    let amount: String?
}

struct DailySpendProvider: TimelineProvider {
    func placeholder(in context: Context) -> DailySpendEntry {
        DailySpendEntry(date: .now, amount: "—")
    }

    func getSnapshot(in context: Context, completion: @escaping (DailySpendEntry) -> Void) {
        let summary = loadSummary()
        completion(DailySpendEntry(date: .now, amount: summary?.dailySpend))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<DailySpendEntry>) -> Void) {
        let summary = loadSummary()
        let entry = DailySpendEntry(date: .now, amount: summary?.dailySpend)
        completion(Timeline(entries: [entry], policy: .after(nextRefresh())))
    }
}

struct DailySpendWidget: Widget {
    let kind = "DailySpendWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: DailySpendProvider()) { entry in
            DailySpendWidgetView(entry: entry)
        }
        .configurationDisplayName("Daily Spend")
        .description("Today's spending total.")
        .supportedFamilies([.accessoryCircular, .accessoryInline])
    }
}

struct DailySpendWidgetView: View {
    let entry: DailySpendEntry

    @Environment(\.widgetFamily) var family

    var body: some View {
        switch family {
        case .accessoryCircular:
            VStack(spacing: 1) {
                Image(systemName: "cart")
                    .font(.caption)
                    .widgetAccentable()
                Text(entry.amount ?? "$0")
                    .font(.caption2.monospacedDigit())
                    .minimumScaleFactor(0.5)
            }

        case .accessoryInline:
            Text("Spent: \(entry.amount ?? "$0") today")

        default:
            Text(entry.amount ?? "$0")
        }
    }
}

// MARK: - Portfolio Widget

struct PortfolioEntry: TimelineEntry {
    let date: Date
    let holdingCount: Int
    let topCommodity: String?
    let topAmount: String?
}

struct PortfolioProvider: TimelineProvider {
    func placeholder(in context: Context) -> PortfolioEntry {
        PortfolioEntry(date: .now, holdingCount: 0, topCommodity: nil, topAmount: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (PortfolioEntry) -> Void) {
        let summary = loadSummary()
        let top = summary?.portfolio.first
        completion(PortfolioEntry(
            date: .now,
            holdingCount: summary?.portfolio.count ?? 0,
            topCommodity: top?.commodity,
            topAmount: top?.amount
        ))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<PortfolioEntry>) -> Void) {
        let summary = loadSummary()
        let top = summary?.portfolio.first
        let entry = PortfolioEntry(
            date: .now,
            holdingCount: summary?.portfolio.count ?? 0,
            topCommodity: top?.commodity,
            topAmount: top?.amount
        )
        completion(Timeline(entries: [entry], policy: .after(nextRefresh())))
    }
}

struct PortfolioWidget: Widget {
    let kind = "PortfolioWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: PortfolioProvider()) { entry in
            PortfolioWidgetView(entry: entry)
        }
        .configurationDisplayName("Portfolio")
        .description("Investment portfolio overview.")
        .supportedFamilies([.accessoryRectangular, .accessoryCircular])
    }
}

struct PortfolioWidgetView: View {
    let entry: PortfolioEntry

    @Environment(\.widgetFamily) var family

    var body: some View {
        switch family {
        case .accessoryRectangular:
            VStack(alignment: .leading, spacing: 2) {
                Text("Portfolio")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .widgetAccentable()
                if let commodity = entry.topCommodity, let amount = entry.topAmount {
                    Text("\(amount) \(commodity)")
                        .font(.headline.monospacedDigit())
                } else {
                    Text("No holdings")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                if entry.holdingCount > 1 {
                    Text("\(entry.holdingCount) holdings")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

        case .accessoryCircular:
            VStack(spacing: 1) {
                Image(systemName: "chart.pie")
                    .font(.caption)
                    .widgetAccentable()
                Text("\(entry.holdingCount)")
                    .font(.caption2.monospacedDigit())
            }

        default:
            Text("\(entry.holdingCount) holdings")
        }
    }
}
