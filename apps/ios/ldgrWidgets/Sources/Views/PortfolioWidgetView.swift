import SwiftUI
import WidgetKit

struct PortfolioWidgetView: View {
    let entry: PortfolioEntry

    var body: some View {
        if entry.isLocked {
            LockedPlaceholderView()
        } else if entry.holdings.isEmpty {
            NoDataPlaceholderView(label: "Portfolio")
        } else {
            holdingsLayout
        }
    }

    private var holdingsLayout: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("Portfolio", systemImage: "chart.pie")
                .font(.caption)
                .foregroundStyle(.secondary)

            ForEach(
                entry.holdings.prefix(5),
                id: \.commodity
            ) { holding in
                HStack {
                    Text(holding.commodity)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .frame(width: 50, alignment: .leading)
                    Spacer()
                    Text(holding.amount)
                        .font(.callout)
                        .fontWeight(.medium)
                        .monospacedDigit()
                }
            }

            if entry.holdings.count > 5 {
                Text("+\(entry.holdings.count - 5) more")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(4)
    }
}
