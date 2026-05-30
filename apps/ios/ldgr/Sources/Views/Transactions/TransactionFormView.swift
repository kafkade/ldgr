import SwiftUI
import LdgrSwift

/// Form for adding a new transaction or correcting an existing one.
///
/// When correcting: creates the new transaction first, then deletes the old one.
/// This ordering ensures no data loss if the second operation fails.
struct TransactionFormView: View {
    let client: LdgrClient
    let store: VaultDataStore
    let editingTransaction: Transaction?

    @Environment(\.dismiss) private var dismiss

    @State private var date = Date()
    @State private var txDescription = ""
    @State private var status: TransactionKind = .unmarked
    @State private var postingRows: [PostingRow] = []
    @State private var isSaving = false
    @State private var errorMessage: String?

    private var isEditing: Bool { editingTransaction != nil }

    private var isValid: Bool {
        !txDescription.trimmingCharacters(in: .whitespaces).isEmpty
            && postingRows.count >= 2
            && postingRows.allSatisfy { !$0.accountId.isEmpty }
            && postingRows.allSatisfy { row in
                row.amount.isEmpty || Decimal(string: row.amount) != nil
            }
    }

    var body: some View {
        NavigationStack {
            Form {
                detailsSection
                postingsSection
                if let errorMessage {
                    Section {
                        Label(errorMessage, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.red)
                            .font(.caption)
                    }
                }
            }
            .navigationTitle(isEditing ? "Correct Transaction" : "New Transaction")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(isEditing ? "Save" : "Add") {
                        Task { await saveTransaction() }
                    }
                    .disabled(!isValid || isSaving)
                }
            }
            .onAppear { prefillFromEditing() }
            .interactiveDismissDisabled(isSaving)
        }
    }

    // MARK: - Details Section

    private var detailsSection: some View {
        Section("Details") {
            DatePicker("Date", selection: $date, displayedComponents: .date)

            TextField("Description", text: $txDescription)
                .textContentType(.name)
                .autocorrectionDisabled()

            Picker("Status", selection: $status) {
                Text("Unmarked").tag(TransactionKind.unmarked)
                Text("Pending").tag(TransactionKind.pending)
                Text("Cleared").tag(TransactionKind.cleared)
            }
        }
    }

    // MARK: - Postings Section

    private var postingsSection: some View {
        Section {
            ForEach($postingRows) { $row in
                PostingRowView(
                    row: $row,
                    accounts: store.accounts
                )
            }
            .onDelete { indices in
                postingRows.remove(atOffsets: indices)
            }

            Button {
                postingRows.append(PostingRow())
            } label: {
                Label("Add Posting", systemImage: "plus.circle")
            }
        } header: {
            HStack {
                Text("Postings")
                Spacer()
                if postingRows.count < 2 {
                    Text("At least 2 required")
                        .font(.caption2)
                        .foregroundStyle(.red)
                }
            }
        }
    }

    // MARK: - Save

    private func saveTransaction() async {
        isSaving = true
        defer { isSaving = false }
        errorMessage = nil

        let dateString = formatDate(date)
        let postings = postingRows.map { row in
            NewPosting(
                accountId: row.accountId,
                amount: row.amount.isEmpty ? nil : row.amount,
                commodity: row.commodity.isEmpty ? nil : row.commodity
            )
        }

        do {
            // Add new transaction first (safe ordering for corrections)
            _ = try client.addTransaction(
                date: dateString,
                description: txDescription.trimmingCharacters(in: .whitespaces),
                status: status,
                postings: postings
            )

            // If correcting, delete the old transaction
            if let old = editingTransaction {
                try client.deleteTransaction(id: old.id)
            }

            await store.reload(client: client)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Prefill

    private func prefillFromEditing() {
        if postingRows.isEmpty && editingTransaction == nil {
            // New transaction: start with 2 empty rows
            postingRows = [PostingRow(), PostingRow()]
        }

        guard let tx = editingTransaction else { return }

        // Parse date
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        if let parsed = formatter.date(from: tx.date) {
            date = parsed
        }

        txDescription = tx.description
        status = tx.status

        postingRows = tx.postings.map { p in
            PostingRow(
                accountId: p.accountId,
                amount: p.amount ?? "",
                commodity: p.commodity ?? ""
            )
        }
    }

    private func formatDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }
}

// MARK: - Posting Row Model

struct PostingRow: Identifiable {
    let id = UUID()
    var accountId: String = ""
    var amount: String = ""
    var commodity: String = ""
}

// MARK: - Posting Row View

private struct PostingRowView: View {
    @Binding var row: PostingRow
    let accounts: [Account]

    var body: some View {
        VStack(spacing: 8) {
            // Account picker
            Picker("Account", selection: $row.accountId) {
                Text("Select account…").tag("")
                ForEach(accountsByType, id: \.type) { group in
                    Section(group.type.rawValue.capitalized) {
                        ForEach(group.accounts) { account in
                            Text(account.name).tag(account.id)
                        }
                    }
                }
            }
            .pickerStyle(.menu)

            HStack(spacing: 8) {
                TextField("Amount", text: $row.amount)
                    .keyboardType(.decimalPad)
                    .font(.body.monospacedDigit())
                    .frame(maxWidth: .infinity)

                TextField("Commodity", text: $row.commodity)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.characters)
                    .frame(width: 60)
            }
        }
        .padding(.vertical, 4)
    }

    private var accountsByType: [(type: AccountKind, accounts: [Account])] {
        let grouped = Dictionary(grouping: accounts) { $0.type }
        let order: [AccountKind] = [.asset, .liability, .income, .expense, .equity]
        return order.compactMap { kind in
            guard let accts = grouped[kind], !accts.isEmpty else { return nil }
            return (type: kind, accounts: accts.sorted { $0.name < $1.name })
        }
    }
}
