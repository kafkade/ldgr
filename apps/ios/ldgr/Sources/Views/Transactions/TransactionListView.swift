import SwiftUI
import LdgrSwift

/// Transaction list with search, filter, pull-to-refresh, swipe-to-delete, and add.
struct TransactionListView: View {
    let store: VaultDataStore
    let client: LdgrClient

    @State private var searchText = ""
    @State private var statusFilter: TransactionKind?
    @State private var presentedForm: FormPresentation?
    @State private var errorMessage: String?

    enum FormPresentation: Identifiable {
        case add
        case edit(Transaction)

        var id: String {
            switch self {
            case .add: return "add"
            case .edit(let tx): return tx.id
            }
        }
    }

    private var filteredTransactions: [Transaction] {
        var result = store.sortedTransactions

        if let filter = statusFilter {
            result = result.filter { $0.status == filter }
        }

        if !searchText.isEmpty {
            let query = searchText.lowercased()
            result = result.filter { tx in
                tx.description.lowercased().contains(query)
                    || tx.date.contains(query)
                    || tx.postings.contains { p in
                        let accountName = store.account(byId: p.accountId)?.name ?? ""
                        return accountName.lowercased().contains(query)
                    }
            }
        }

        return result
    }

    var body: some View {
        List {
            if store.isLoading && store.transactions.isEmpty {
                Section {
                    ProgressView("Loading…")
                        .frame(maxWidth: .infinity)
                }
            } else if filteredTransactions.isEmpty {
                Section {
                    if store.transactions.isEmpty {
                        emptyState
                    } else {
                        noResultsView
                    }
                }
            } else {
                transactionSections
            }
        }
        .navigationTitle("Transactions")
        .searchable(text: $searchText, prompt: "Search transactions")
        .refreshable {
            await store.reload(client: client)
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    presentedForm = .add
                } label: {
                    Image(systemName: "plus")
                }
            }
            ToolbarItem(placement: .secondaryAction) {
                filterMenu
            }
        }
        .sheet(item: $presentedForm) { form in
            switch form {
            case .add:
                TransactionFormView(
                    client: client,
                    store: store,
                    editingTransaction: nil
                )
            case .edit(let tx):
                TransactionFormView(
                    client: client,
                    store: store,
                    editingTransaction: tx
                )
            }
        }
        .alert("Error", isPresented: .constant(errorMessage != nil)) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    // MARK: - Empty States

    private var emptyState: some View {
        VStack(spacing: 16) {
            Image(systemName: "list.bullet.rectangle")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("No Transactions")
                .font(.title3.weight(.semibold))
            Text("Tap + to add your first transaction.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }

    private var noResultsView: some View {
        VStack(spacing: 12) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 32))
                .foregroundStyle(.secondary)
            Text("No matching transactions")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }

    // MARK: - Transaction Sections (grouped by date)

    private var transactionSections: some View {
        let grouped = Dictionary(grouping: filteredTransactions) { tx in
            String(tx.date.prefix(7)) // group by YYYY-MM
        }
        let sortedKeys = grouped.keys.sorted(by: >)

        return ForEach(sortedKeys, id: \.self) { month in
            Section(formatMonthHeader(month)) {
                ForEach(grouped[month] ?? []) { tx in
                    TransactionRow(transaction: tx, store: store)
                        .contentShape(Rectangle())
                        .onTapGesture {
                            presentedForm = .edit(tx)
                        }
                        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                            Button(role: .destructive) {
                                deleteTransaction(tx)
                            } label: {
                                Label("Delete", systemImage: "trash")
                            }
                        }
                }
            }
        }
    }

    // MARK: - Filter Menu

    private var filterMenu: some View {
        Menu {
            Button {
                statusFilter = nil
            } label: {
                Label("All", systemImage: statusFilter == nil ? "checkmark" : "")
            }
            Divider()
            Button {
                statusFilter = .cleared
            } label: {
                Label("Cleared", systemImage: statusFilter == .cleared ? "checkmark" : "")
            }
            Button {
                statusFilter = .pending
            } label: {
                Label("Pending", systemImage: statusFilter == .pending ? "checkmark" : "")
            }
            Button {
                statusFilter = .unmarked
            } label: {
                Label("Unmarked", systemImage: statusFilter == .unmarked ? "checkmark" : "")
            }
        } label: {
            Image(systemName: statusFilter != nil ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
    }

    // MARK: - Actions

    private func deleteTransaction(_ tx: Transaction) {
        Task {
            do {
                try client.deleteTransaction(id: tx.id)
                await store.reload(client: client)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    // MARK: - Helpers

    private func formatMonthHeader(_ yearMonth: String) -> String {
        let parts = yearMonth.split(separator: "-")
        guard parts.count == 2,
              let year = Int(parts[0]),
              let month = Int(parts[1]) else {
            return yearMonth
        }

        let formatter = DateFormatter()
        formatter.dateFormat = "MMMM yyyy"
        var components = DateComponents()
        components.year = year
        components.month = month
        components.day = 1
        if let date = Calendar.current.date(from: components) {
            return formatter.string(from: date)
        }
        return yearMonth
    }
}
