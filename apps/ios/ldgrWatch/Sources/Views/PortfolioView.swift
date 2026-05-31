import SwiftUI

/// Portfolio holdings summary grouped by commodity.
struct PortfolioView: View {
    let holdings: [WatchSummary.PortfolioHolding]

    private var grouped: [(commodity: String, total: Decimal)] {
        var totals: [String: Decimal] = [:]
        for h in holdings {
            totals[h.commodity, default: 0] += Decimal(string: h.amount) ?? 0
        }
        return totals.map { (commodity: $0.key, total: $0.value) }
            .sorted { $0.total > $1.total }
    }

    var body: some View {
        List {
            if holdings.isEmpty {
                Text("No holdings")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(grouped, id: \.commodity) { entry in
                    HStack {
                        Text(entry.commodity)
                            .font(.caption.weight(.medium))
                        Spacer()
                        Text(formatDecimal(entry.total))
                            .font(.body.monospacedDigit())
                    }
                }
            }
        }
        .navigationTitle("Portfolio")
    }

    private func formatDecimal(_ value: Decimal) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 6
        return formatter.string(from: value as NSDecimalNumber) ?? "\(value)"
    }
}
