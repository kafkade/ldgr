import SwiftUI
import LdgrSwift

/// Dashboard tab: net worth, recent transactions, and quick stats.
struct DashboardView: View {
    let store: VaultDataStore
    let client: LdgrClient
    @Bindable var appState: AppState

    var body: some View {
        List {
            if store.isLoading {
                Section {
                    ProgressView("Loading…")
                        .frame(maxWidth: .infinity)
                }
            } else if store.accounts.isEmpty {
                emptyState
            } else {
                netWorthSection
                quickStatsSection
                recentTransactionsSection
                expenseBreakdownSection
            }
        }
        .navigationTitle("Dashboard")
        .refreshable {
            // TODO: Trigger sync when sync API is available
            await store.reload(client: client)
        }
    }

    // MARK: - Empty State

    private var emptyState: some View {
        Section {
            VStack(spacing: 16) {
                Image(systemName: "chart.bar.fill")
                    .font(.system(size: 48))
                    .foregroundStyle(.secondary)
                Text("No Data Yet")
                    .font(.title3.weight(.semibold))
                Text("Add accounts and transactions to see your financial dashboard.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 24)
        }
    }

    // MARK: - Net Worth

    private var netWorthSection: some View {
        Section {
            ForEach(store.netWorthByCommodity, id: \.commodity) { entry in
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Net Worth")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(formatDecimal(entry.amount))
                            .font(.title.weight(.bold).monospacedDigit())
                            .foregroundStyle(entry.amount >= 0 ? AnyShapeStyle(.primary) : AnyShapeStyle(.red))
                    }
                    Spacer()
                    Text(entry.commodity)
                        .font(.headline)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
            }

            if store.netWorthByCommodity.isEmpty {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Net Worth")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("0.00")
                            .font(.title.weight(.bold).monospacedDigit())
                    }
                    Spacer()
                }
                .padding(.vertical, 4)
            }
        }
    }

    // MARK: - Quick Stats

    private var quickStatsSection: some View {
        Section("Overview") {
            HStack {
                StatCard(
                    title: "Accounts",
                    value: "\(store.accounts.count)",
                    icon: "building.columns",
                    color: .blue
                )
                StatCard(
                    title: "Transactions",
                    value: "\(store.transactions.count)",
                    icon: "arrow.left.arrow.right",
                    color: .green
                )
            }
            .listRowInsets(EdgeInsets())
            .listRowBackground(Color.clear)
        }
    }

    // MARK: - Recent Transactions

    private var recentTransactionsSection: some View {
        Section("Recent Transactions") {
            let recent = Array(store.sortedTransactions.prefix(5))
            if recent.isEmpty {
                Text("No transactions yet")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(recent) { tx in
                    TransactionRow(transaction: tx, store: store)
                }
            }
        }
    }

    // MARK: - Expense Breakdown

    private var expenseBreakdownSection: some View {
        Section("Expense Accounts") {
            let expenseBalances = store.balances.filter { entry in
                store.accountTypesByName[entry.account] == .expense
            }

            if expenseBalances.isEmpty {
                Text("No expense accounts")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(expenseBalances.prefix(5), id: \.account) { entry in
                    HStack {
                        Text(shortAccountName(entry.account))
                            .font(.subheadline)
                        Spacer()
                        Text("\(entry.amount) \(entry.commodity)")
                            .font(.subheadline.monospacedDigit())
                            .foregroundStyle(.orange)
                    }
                }
            }
        }
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
        let components = name.split(separator: ":")
        return components.count > 1
            ? String(components.dropFirst().joined(separator: ":"))
            : name
    }
}

// MARK: - Stat Card

private struct StatCard: View {
    let title: String
    let value: String
    let icon: String
    let color: Color

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon)
                .font(.title2)
                .foregroundStyle(color)
            Text(value)
                .font(.title2.weight(.bold).monospacedDigit())
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding()
        .background(color.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .padding(4)
    }
}

// MARK: - Transaction Row (shared)

/// Compact transaction row used in dashboard and transaction list.
struct TransactionRow: View {
    let transaction: Transaction
    let store: VaultDataStore

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                statusIcon
                Text(transaction.description)
                    .font(.subheadline.weight(.medium))
                    .lineLimit(1)
                Spacer()
                Text(transaction.date)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let primary = primaryPosting {
                HStack {
                    let accountName = store.account(byId: primary.accountId)?.name ?? primary.accountId
                    Text(accountName)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer()
                    if let amount = primary.amount, let commodity = primary.commodity {
                        Text("\(amount) \(commodity)")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(
                                amount.hasPrefix("-") ? .red : .green
                            )
                    }
                }
            }
        }
        .padding(.vertical, 2)
    }

    private var statusIcon: some View {
        Group {
            switch transaction.status {
            case .cleared:
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            case .pending:
                Image(systemName: "clock.fill")
                    .foregroundStyle(.orange)
            case .unmarked:
                Image(systemName: "circle")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.caption)
    }

    private var primaryPosting: Posting? {
        // Show the first posting with a positive amount, or just the first one
        transaction.postings.first { p in
            guard let amount = p.amount else { return false }
            return !amount.hasPrefix("-")
        } ?? transaction.postings.first
    }
}
