import SwiftUI
import LdgrSwift

/// Main view after vault unlock — shows accounts and balances.
struct HomeView: View {
    @Bindable var appState: AppState
    let client: LdgrClient

    @State private var accounts: [Account] = []
    @State private var balances: [BalanceEntry] = []
    @State private var vaultName: String = ""
    @State private var showSettings = false
    @State private var isLoading = true

    var body: some View {
        NavigationStack {
            Group {
                if isLoading {
                    ProgressView("Loading…")
                } else if accounts.isEmpty {
                    emptyStateView
                } else {
                    accountListView
                }
            }
            .navigationTitle(vaultName)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showSettings = true
                    } label: {
                        Image(systemName: "gearshape")
                    }
                }
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        client.close()
                        appState.transitionToLocked()
                    } label: {
                        Image(systemName: "lock")
                    }
                }
            }
            .sheet(isPresented: $showSettings) {
                SettingsView(appState: appState, client: client)
            }
            .task {
                await loadData()
            }
        }
    }

    // MARK: - Subviews

    private var emptyStateView: some View {
        VStack(spacing: 16) {
            Image(systemName: "building.columns")
                .font(.system(size: 56))
                .foregroundStyle(.secondary)
            Text("No Accounts Yet")
                .font(.title3.weight(.semibold))
            Text("Add accounts and transactions using the CLI, then they'll appear here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 40)
        }
    }

    private var accountListView: some View {
        List {
            if !balances.isEmpty {
                Section("Balances") {
                    ForEach(balances, id: \.account) { entry in
                        HStack {
                            Text(entry.account)
                                .font(.subheadline)
                            Spacer()
                            Text("\(entry.amount) \(entry.commodity)")
                                .font(.subheadline.monospacedDigit())
                                .foregroundStyle(
                                    entry.amount.hasPrefix("-") ? .red : .primary
                                )
                        }
                    }
                }
            }

            Section("Accounts (\(accounts.count))") {
                ForEach(accounts) { account in
                    HStack {
                        Label {
                            Text(account.name)
                                .font(.subheadline)
                        } icon: {
                            Image(systemName: iconForAccountType(account.type))
                                .foregroundStyle(colorForAccountType(account.type))
                        }
                        Spacer()
                        if let commodity = account.commodity {
                            Text(commodity)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Data Loading

    private func loadData() async {
        defer { isLoading = false }
        do {
            vaultName = try client.vaultName()
            accounts = try client.listAccounts()
            balances = try client.balance()
        } catch {
            appState.setError(error.localizedDescription)
        }
    }

    // MARK: - Helpers

    private func iconForAccountType(_ type: AccountKind) -> String {
        switch type {
        case .asset: return "banknote"
        case .liability: return "creditcard"
        case .income: return "arrow.down.circle"
        case .expense: return "arrow.up.circle"
        case .equity: return "chart.pie"
        }
    }

    private func colorForAccountType(_ type: AccountKind) -> Color {
        switch type {
        case .asset: return .green
        case .liability: return .red
        case .income: return .blue
        case .expense: return .orange
        case .equity: return .purple
        }
    }
}
