import SwiftUI
import LdgrSwift

/// Register view: all transactions for a specific account.
struct AccountRegisterView: View {
    let account: Account
    let store: VaultDataStore
    let client: LdgrClient

    @State private var editingTransaction: Transaction?

    private var registerTransactions: [Transaction] {
        store.transactions(forAccountId: account.id)
    }

    var body: some View {
        List {
            balanceHeader

            if registerTransactions.isEmpty {
                Section {
                    VStack(spacing: 12) {
                        Image(systemName: "doc.text")
                            .font(.system(size: 32))
                            .foregroundStyle(.secondary)
                        Text("No transactions for this account")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 16)
                }
            } else {
                Section("Transactions (\(registerTransactions.count))") {
                    ForEach(registerTransactions) { tx in
                        registerRow(tx)
                            .contentShape(Rectangle())
                            .onTapGesture {
                                editingTransaction = tx
                            }
                    }
                }
            }
        }
        .navigationTitle(account.name)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable {
            await store.reload(client: client)
        }
        .sheet(item: $editingTransaction) { tx in
            TransactionFormView(
                client: client,
                store: store,
                editingTransaction: tx
            )
        }
    }

    // MARK: - Balance Header

    private var balanceHeader: some View {
        Section {
            if let balance = store.balance(forAccount: account.name) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Balance")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("\(balance.amount) \(balance.commodity)")
                            .font(.title2.weight(.bold).monospacedDigit())
                            .foregroundStyle(
                                balance.amount.hasPrefix("-") ? .red : .primary
                            )
                    }
                    Spacer()
                    typeLabel
                }
            } else {
                HStack {
                    Text("No balance data")
                        .foregroundStyle(.secondary)
                    Spacer()
                    typeLabel
                }
            }
        }
    }

    private var typeLabel: some View {
        Text(account.type.rawValue.capitalized)
            .font(.caption)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(colorForType(account.type).opacity(0.15))
            .foregroundStyle(colorForType(account.type))
            .clipShape(Capsule())
    }

    // MARK: - Register Row

    private func registerRow(_ tx: Transaction) -> some View {
        let posting = tx.postings.first { $0.accountId == account.id }
        let counterparties = tx.postings
            .filter { $0.accountId != account.id }
            .compactMap { store.account(byId: $0.accountId)?.name }

        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                statusIcon(tx.status)
                Text(tx.description)
                    .font(.subheadline.weight(.medium))
                    .lineLimit(1)
                Spacer()
                if let amount = posting?.amount, let commodity = posting?.commodity {
                    Text("\(amount) \(commodity)")
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(amount.hasPrefix("-") ? .red : .green)
                }
            }

            HStack {
                Text(tx.date)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if !counterparties.isEmpty {
                    Text("→ \(counterparties.joined(separator: ", "))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
        .padding(.vertical, 2)
    }

    private func statusIcon(_ status: TransactionKind) -> some View {
        Group {
            switch status {
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
