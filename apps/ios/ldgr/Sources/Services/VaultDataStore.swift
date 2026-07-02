import Foundation
import LdgrSwift

/// Shared data store for vault contents, observed by all tabs.
///
/// Centralizes vault reads so mutations (add/delete transaction) can
/// trigger a single reload that updates every visible surface.
@MainActor
@Observable
final class VaultDataStore {
    private(set) var accounts: [Account] = []
    private(set) var transactions: [Transaction] = []
    private(set) var balances: [BalanceEntry] = []
    private(set) var vaultName: String = ""
    private(set) var isLoading = false
    var errorMessage: String?

    /// Full reload from the encrypted vault.
    ///
    /// Dispatches vault I/O to a background thread so the main actor
    /// is never blocked by crypto or SQLite operations.
    func reload(client: LdgrClient) async {
        isLoading = true
        defer { isLoading = false }

        do {
            let result = try await Task.detached {
                let name = try client.vaultName()
                let accts = try client.listAccounts()
                let txns = try client.listTransactions()
                let bals = try client.balance()
                return (name, accts, txns, bals)
            }.value

            vaultName = result.0
            accounts = result.1
            transactions = result.2
            balances = result.3
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Derived Data

    /// Account lookup by ID.
    func account(byId id: String) -> Account? {
        accounts.first { $0.id == id }
    }

    /// Account lookup by name.
    func account(byName name: String) -> Account? {
        accounts.first { $0.name == name }
    }

    /// Map from account name → AccountKind for balance classification.
    var accountTypesByName: [String: AccountKind] {
        Dictionary(uniqueKeysWithValues: accounts.map { ($0.name, $0.type) })
    }

    /// Transactions sorted by date descending.
    var sortedTransactions: [Transaction] {
        transactions.sorted { $0.date > $1.date }
    }

    /// Net worth grouped by commodity: sum(assets) - sum(liabilities).
    var netWorthByCommodity: [(commodity: String, amount: Decimal)] {
        let types = accountTypesByName
        var totals: [String: Decimal] = [:]

        for entry in balances {
            guard let kind = types[entry.account] else { continue }
            guard kind == .asset || kind == .liability else { continue }

            let amount = Decimal(string: entry.amount) ?? 0
            let contribution = kind == .asset ? amount : -amount
            totals[entry.commodity, default: 0] += contribution
        }

        return totals.map { (commodity: $0.key, amount: $0.value) }
            .sorted { $0.commodity < $1.commodity }
    }

    /// Accounts grouped by type.
    var accountsByType: [(type: AccountKind, accounts: [Account])] {
        let grouped = Dictionary(grouping: accounts) { $0.type }
        let order: [AccountKind] = [.asset, .liability, .income, .expense, .equity]
        return order.compactMap { kind in
            guard let accts = grouped[kind], !accts.isEmpty else { return nil }
            return (type: kind, accounts: accts.sorted { $0.name < $1.name })
        }
    }

    /// Balance for a specific account.
    func balance(forAccount name: String) -> BalanceEntry? {
        balances.first { $0.account == name }
    }

    /// Transactions that involve a specific account (by ID).
    func transactions(forAccountId id: String) -> [Transaction] {
        sortedTransactions.filter { tx in
            tx.postings.contains { $0.accountId == id }
        }
    }
}
