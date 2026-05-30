import SwiftUI
import LdgrSwift

/// Accounts grouped by type with balances. Tap to view register.
struct AccountListView: View {
    let store: VaultDataStore
    let client: LdgrClient

    var body: some View {
        List {
            if store.isLoading && store.accounts.isEmpty {
                Section {
                    ProgressView("Loading…")
                        .frame(maxWidth: .infinity)
                }
            } else if store.accounts.isEmpty {
                Section {
                    emptyState
                }
            } else {
                ForEach(store.accountsByType, id: \.type) { group in
                    Section {
                        ForEach(group.accounts) { account in
                            NavigationLink {
                                AccountRegisterView(
                                    account: account,
                                    store: store,
                                    client: client
                                )
                            } label: {
                                accountRow(account)
                            }
                        }
                    } header: {
                        Label(
                            typeName(group.type),
                            systemImage: iconForType(group.type)
                        )
                        .foregroundStyle(colorForType(group.type))
                    }
                }
            }
        }
        .navigationTitle("Accounts")
        .refreshable {
            await store.reload(client: client)
        }
    }

    // MARK: - Subviews

    private var emptyState: some View {
        VStack(spacing: 16) {
            Image(systemName: "building.columns")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("No Accounts")
                .font(.title3.weight(.semibold))
            Text("Add accounts using the CLI or the Transactions tab.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }

    private func accountRow(_ account: Account) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(account.name)
                    .font(.subheadline)
                if let commodity = account.commodity {
                    Text(commodity)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if let balance = store.balance(forAccount: account.name) {
                Text("\(balance.amount) \(balance.commodity)")
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(
                        balance.amount.hasPrefix("-") ? .red : .primary
                    )
            }
        }
        .padding(.vertical, 2)
    }

    // MARK: - Helpers

    private func typeName(_ type: AccountKind) -> String {
        switch type {
        case .asset: return "Assets"
        case .liability: return "Liabilities"
        case .income: return "Income"
        case .expense: return "Expenses"
        case .equity: return "Equity"
        }
    }

    private func iconForType(_ type: AccountKind) -> String {
        switch type {
        case .asset: return "banknote"
        case .liability: return "creditcard"
        case .income: return "arrow.down.circle"
        case .expense: return "arrow.up.circle"
        case .equity: return "chart.pie"
        }
    }

    private func colorForType(_ type: AccountKind) -> Color {
        switch type {
        case .asset: return .green
        case .liability: return .red
        case .income: return .blue
        case .expense: return .orange
        case .equity: return .purple
        }
    }
}
