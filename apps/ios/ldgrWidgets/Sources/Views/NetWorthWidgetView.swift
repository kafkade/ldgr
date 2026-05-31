import SwiftUI
import WidgetKit

struct NetWorthWidgetView: View {
    let entry: NetWorthEntry

    @Environment(\.widgetFamily) var family

    var body: some View {
        if entry.isLocked {
            LockedPlaceholderView()
        } else if entry.netWorth.isEmpty {
            NoDataPlaceholderView(label: "Net Worth")
        } else {
            switch family {
            case .systemSmall:
                smallLayout
            default:
                mediumLayout
            }
        }
    }

    private var smallLayout: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("Net Worth", systemImage: "banknote")
                .font(.caption)
                .foregroundStyle(.secondary)

            if let primary = entry.netWorth.first {
                Text(primary.amount)
                    .font(.title2)
                    .fontWeight(.bold)
                    .minimumScaleFactor(0.6)
                    .lineLimit(1)

                Text(primary.commodity)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            if entry.netWorth.count > 1 {
                Text("+\(entry.netWorth.count - 1) more")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(4)
    }

    private var mediumLayout: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("Net Worth", systemImage: "banknote")
                .font(.caption)
                .foregroundStyle(.secondary)

            ForEach(entry.netWorth.prefix(4), id: \.commodity) { item in
                HStack {
                    Text(item.commodity)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .frame(width: 40, alignment: .leading)
                    Spacer()
                    Text(item.amount)
                        .font(.callout)
                        .fontWeight(.semibold)
                        .monospacedDigit()
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(4)
    }
}

// MARK: - Shared Placeholder Views

struct LockedPlaceholderView: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "lock.fill")
                .font(.title2)
                .foregroundStyle(.secondary)
            Text("Unlock ldgr to view")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct NoDataPlaceholderView: View {
    let label: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "chart.bar.xaxis")
                .font(.title2)
                .foregroundStyle(.secondary)
            Text("No \(label) data")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
