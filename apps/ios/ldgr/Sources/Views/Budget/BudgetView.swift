import SwiftUI
import LdgrSwift

/// Budget view: expense category spending with progress bars.
///
/// Uses the current calendar month for date-filtered balances.
struct BudgetView: View {
    let store: VaultDataStore
    let client: LdgrClient

    @State private var monthlyBalances: [BalanceEntry] = []
    @State private var isLoadingMonthly = false

    private var expenseBalances: [BalanceEntry] {
        let types = store.accountTypesByName
        return monthlyBalances.filter { types[$0.account] == .expense }
    }

    private var totalExpense: Decimal {
        expenseBalances.reduce(Decimal.zero) { total, entry in
            total + (Decimal(string: entry.amount) ?? 0)
        }
    }

    private var maxExpense: Decimal {
        expenseBalances.reduce(Decimal.zero) { current, entry in
            max(current, abs(Decimal(string: entry.amount) ?? 0))
        }
    }

    var body: some View {
        List {
            if store.isLoading && store.accounts.isEmpty {
                Section {
                    ProgressView("Loading…")
                        .frame(maxWidth: .infinity)
                }
            } else if expenseBalances.isEmpty && !isLoadingMonthly {
                Section {
                    emptyState
                }
            } else {
                summarySection
                categorySection
            }
        }
        .navigationTitle("Budget")
        .refreshable {
            await store.reload(client: client)
            await loadMonthlyBalances()
        }
        .task {
            await loadMonthlyBalances()
        }
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 16) {
            Image(systemName: "gauge.with.dots.needle.67percent")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("No Expense Data")
                .font(.title3.weight(.semibold))
            Text("Expense account spending for the current month will appear here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }

    // MARK: - Summary

    private var summarySection: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text(monthLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline) {
                    Text(formatDecimal(totalExpense))
                        .font(.title.weight(.bold).monospacedDigit())
                    Text("total spending")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Text("\(expenseBalances.count) categories")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(.vertical, 4)
        }
    }

    // MARK: - Categories

    private var categorySection: some View {
        Section("Categories") {
            let sorted = expenseBalances.sorted { a, b in
                abs(Decimal(string: a.amount) ?? 0) > abs(Decimal(string: b.amount) ?? 0)
            }

            ForEach(sorted, id: \.account) { entry in
                let amount = abs(Decimal(string: entry.amount) ?? 0)
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(shortName(entry.account))
                            .font(.subheadline)
                        Spacer()
                        Text("\(entry.amount) \(entry.commodity)")
                            .font(.subheadline.monospacedDigit())
                    }

                    GeometryReader { geo in
                        let fraction = maxExpense > 0
                            ? CGFloat(NSDecimalNumber(decimal: amount / maxExpense).doubleValue)
                            : 0
                        RoundedRectangle(cornerRadius: 4)
                            .fill(.orange.opacity(0.3))
                            .frame(width: geo.size.width)
                            .overlay(alignment: .leading) {
                                RoundedRectangle(cornerRadius: 4)
                                    .fill(.orange)
                                    .frame(width: geo.size.width * min(fraction, 1.0))
                            }
                    }
                    .frame(height: 8)
                }
                .padding(.vertical, 4)
            }
        }
    }

    // MARK: - Data Loading

    private func loadMonthlyBalances() async {
        isLoadingMonthly = true
        defer { isLoadingMonthly = false }

        let (begin, end) = currentMonthRange()

        do {
            monthlyBalances = try await Task.detached {
                try client.balance(
                    accountFilter: nil,
                    beginDate: begin,
                    endDate: end
                )
            }.value
        } catch {
            // Fall back to full balances
            monthlyBalances = store.balances
        }
    }

    // MARK: - Helpers

    private func currentMonthRange() -> (begin: String, end: String) {
        let calendar = Calendar.current
        let now = Date()
        let components = calendar.dateComponents([.year, .month], from: now)
        let startOfMonth = calendar.date(from: components) ?? now

        var nextComponents = DateComponents()
        nextComponents.month = 1
        let startOfNext = calendar.date(byAdding: nextComponents, to: startOfMonth) ?? now

        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"

        return (
            begin: formatter.string(from: startOfMonth),
            end: formatter.string(from: startOfNext)
        )
    }

    private var monthLabel: String {
        let formatter = DateFormatter()
        formatter.dateFormat = "MMMM yyyy"
        return formatter.string(from: Date())
    }

    private func formatDecimal(_ value: Decimal) -> String {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        return formatter.string(from: value as NSDecimalNumber) ?? "\(value)"
    }

    private func shortName(_ name: String) -> String {
        let parts = name.split(separator: ":")
        return parts.count > 1
            ? String(parts.dropFirst().joined(separator: ":"))
            : name
    }
}
