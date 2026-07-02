import SwiftUI
import Charts
import LdgrSwift

/// Investment portfolio view: holdings by commodity and allocation chart.
///
/// Derives investment data from asset accounts that hold non-default
/// commodities (e.g., stocks, ETFs, crypto).
struct InvestmentsView: View {
    let store: VaultDataStore

    private var holdings: [Holding] {
        let types = store.accountTypesByName

        return store.balances.compactMap { entry -> Holding? in
            guard types[entry.account] == .asset else { return nil }
            guard let amount = Decimal(string: entry.amount), amount != 0 else { return nil }
            return Holding(
                account: entry.account,
                amount: amount,
                commodity: entry.commodity
            )
        }
        .sorted { $0.account < $1.account }
    }

    private var allocationByCommodity: [(commodity: String, total: Decimal)] {
        var totals: [String: Decimal] = [:]
        for holding in holdings {
            totals[holding.commodity, default: 0] += abs(holding.amount)
        }
        return totals.map { (commodity: $0.key, total: $0.value) }
            .sorted { $0.total > $1.total }
    }

    var body: some View {
        List {
            if store.isLoading && store.accounts.isEmpty {
                Section {
                    ProgressView("Loading…")
                        .frame(maxWidth: .infinity)
                }
            } else if holdings.isEmpty {
                Section {
                    emptyState
                }
            } else {
                allocationSection
                holdingsSection
            }
        }
        .navigationTitle("Investments")
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 16) {
            Image(systemName: "chart.pie.fill")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("No Investments")
                .font(.title3.weight(.semibold))
            Text("Asset accounts with holdings will appear here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }

    // MARK: - Allocation Chart

    private var allocationSection: some View {
        Section("Allocation") {
            if allocationByCommodity.count > 1 {
                Chart(allocationByCommodity, id: \.commodity) { item in
                    BarMark(
                        x: .value("Amount", NSDecimalNumber(decimal: item.total).doubleValue)
                    )
                    .foregroundStyle(by: .value("Commodity", item.commodity))
                    .cornerRadius(4)
                }
                .chartXAxis(.hidden)
                .frame(height: 40)
                .padding(.vertical, 8)
            }

            ForEach(allocationByCommodity, id: \.commodity) { item in
                HStack {
                    Text(item.commodity)
                        .font(.subheadline.weight(.medium))
                    Spacer()
                    Text(formatDecimal(item.total))
                        .font(.subheadline.monospacedDigit())
                    if allocationByCommodity.count > 1 {
                        Text(allocationPercentage(item.total))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(width: 48, alignment: .trailing)
                    }
                }
            }
        }
    }

    // MARK: - Holdings List

    private var holdingsSection: some View {
        Section("Holdings (\(holdings.count))") {
            ForEach(holdings, id: \.account) { holding in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(shortName(holding.account))
                            .font(.subheadline)
                        Text(holding.commodity)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(formatDecimal(holding.amount))
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(holding.amount >= 0 ? AnyShapeStyle(.primary) : AnyShapeStyle(.red))
                }
                .padding(.vertical, 2)
            }
        }
    }

    // MARK: - Helpers

    private func formatDecimal(_ value: Decimal) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 6
        return formatter.string(from: value as NSDecimalNumber) ?? "\(value)"
    }

    private func allocationPercentage(_ value: Decimal) -> String {
        let grandTotal = allocationByCommodity.reduce(Decimal.zero) { $0 + $1.total }
        guard grandTotal > 0 else { return "0%" }
        let pct = (value / grandTotal) * 100
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 1
        let formatted = formatter.string(from: pct as NSDecimalNumber) ?? "0"
        return "\(formatted)%"
    }

    private func shortName(_ name: String) -> String {
        let parts = name.split(separator: ":")
        return parts.count > 1
            ? String(parts.dropFirst().joined(separator: ":"))
            : name
    }
}

// MARK: - Holding Model

private struct Holding {
    let account: String
    let amount: Decimal
    let commodity: String
}
