#if os(macOS)
import SwiftUI
import UniformTypeIdentifiers
import LdgrSwift

/// Native macOS navigation shell: a `NavigationSplitView` sidebar + detail
/// layout that replaces the iOS `TabView`. It reuses the shared
/// `VaultDataStore`, `SyncManager`, and section content views — only the
/// navigation chrome, menu commands, and keyboard shortcuts are macOS-specific.
struct MacRootView: View {
    @Bindable var appState: AppState
    let client: LdgrClient

    @State private var store = VaultDataStore()
    @State private var syncManager = SyncManager()
    @State private var selection: Section? = .dashboard
    @State private var showNewTransaction = false
    @State private var showSettings = false
    @State private var showImporter = false
    @State private var infoMessage: String?

    @Environment(WatchConnectivityManager.self) private var watchManager
    @Environment(WidgetDataManager.self) private var widgetManager

    /// Sidebar sections. Mirrors the iOS tabs so the view models and content
    /// views are shared verbatim.
    enum Section: String, CaseIterable, Identifiable {
        case dashboard = "Dashboard"
        case transactions = "Transactions"
        case accounts = "Accounts"
        case investments = "Investments"
        case budget = "Budget"

        var id: String { rawValue }

        var icon: String {
            switch self {
            case .dashboard: return "chart.bar.fill"
            case .transactions: return "list.bullet.rectangle"
            case .accounts: return "building.columns.fill"
            case .investments: return "chart.pie.fill"
            case .budget: return "gauge.with.dots.needle.67percent"
            }
        }
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            NavigationStack {
                detail(for: selection ?? .dashboard)
                    .toolbar { detailToolbar }
            }
        }
        .frame(minWidth: 820, minHeight: 520)
        .task {
            await store.reload(client: client)
            syncManager.configure(client: client)
            await syncManager.refreshStatus(client: client)
            await watchManager.sendUpdate(from: store, client: client)
            await widgetManager.sendUpdate(from: store, client: client)
        }
        .onChange(of: store.isLoading) { oldValue, newValue in
            if oldValue && !newValue {
                Task {
                    await watchManager.sendUpdate(from: store, client: client)
                    await widgetManager.sendUpdate(from: store, client: client)
                }
            }
        }
        .sheet(isPresented: $showNewTransaction) {
            TransactionFormView(client: client, store: store, editingTransaction: nil)
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                SyncSettingsView(client: client, syncManager: syncManager)
            }
        }
        .fileImporter(
            isPresented: $showImporter,
            allowedContentTypes: MacRootView.journalContentTypes,
            allowsMultipleSelection: false,
            onCompletion: handleImportResult
        )
        .alert("ldgr", isPresented: infoAlertBinding) {
            Button("OK") { infoMessage = nil }
        } message: {
            Text(infoMessage ?? "")
        }
        // Expose this window's actions to the macOS menu bar while it is key.
        .focusedSceneValue(\.ldgrWindowActions, windowActions)
    }

    // MARK: - Sidebar

    private var sidebar: some View {
        List(Section.allCases, selection: $selection) { section in
            Label(section.rawValue, systemImage: section.icon)
                .tag(section)
        }
        .navigationSplitViewColumnWidth(min: 200, ideal: 220, max: 320)
        .navigationTitle(store.vaultName.isEmpty ? "ldgr" : store.vaultName)
        .safeAreaInset(edge: .bottom) {
            sidebarFooter
        }
    }

    private var sidebarFooter: some View {
        HStack(spacing: 10) {
            SyncStatusIndicator(syncManager: syncManager)
            Text(syncFooterText)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer()
            Button {
                Task { await syncManager.sync(client: client) }
            } label: {
                Image(systemName: "arrow.triangle.2.circlepath")
            }
            .buttonStyle(.borderless)
            .disabled(syncManager.isSyncing)
            .help("Sync Now (⌘R)")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var syncFooterText: String {
        if syncManager.isSyncing { return "Syncing…" }
        guard let status = syncManager.status else { return "Not synced" }
        if status.unresolvedConflictCount > 0 {
            return "\(status.unresolvedConflictCount) conflicts"
        }
        if status.pendingEventCount > 0 {
            return "\(status.pendingEventCount) pending"
        }
        return "Up to date"
    }

    // MARK: - Detail

    @ViewBuilder
    private func detail(for section: Section) -> some View {
        switch section {
        case .dashboard:
            DashboardView(store: store, client: client, appState: appState)
        case .transactions:
            TransactionListView(store: store, client: client)
        case .accounts:
            AccountListView(store: store, client: client)
        case .investments:
            InvestmentsView(store: store)
        case .budget:
            BudgetView(store: store, client: client)
        }
    }

    @ToolbarContentBuilder
    private var detailToolbar: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button {
                showSettings = true
            } label: {
                Image(systemName: "gearshape")
            }
            .help("Sync & Settings")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                lockVault()
            } label: {
                Image(systemName: "lock")
            }
            .help("Lock Vault (⌘L)")
        }
    }

    // MARK: - Menu actions

    private var windowActions: LdgrWindowActions {
        LdgrWindowActions(
            newTransaction: { showNewTransaction = true },
            importJournal: { showImporter = true },
            lock: { lockVault() },
            sync: { Task { await syncManager.sync(client: client) } },
            canSync: !syncManager.isSyncing
        )
    }

    private func lockVault() {
        guard appState.status == .unlocked else { return }
        client.close()
        widgetManager.clearOnLock()
        appState.transitionToLocked()
    }

    // MARK: - Import

    /// Content types offered by the import picker. hledger journals are plain
    /// text; `.journal`/`.ledger` extensions are matched loosely.
    private static let journalContentTypes: [UTType] = {
        var types: [UTType] = [.plainText, .text, .data]
        if let journal = UTType(filenameExtension: "journal") { types.insert(journal, at: 0) }
        if let ledger = UTType(filenameExtension: "ledger") { types.insert(ledger, at: 0) }
        return types
    }()

    private func handleImportResult(_ result: Result<[URL], Error>) {
        switch result {
        case .success(let urls):
            guard let url = urls.first else { return }
            Task { await importJournal(from: url) }
        case .failure(let error):
            infoMessage = error.localizedDescription
        }
    }

    /// Read the selected journal file and apply it to the vault, then refresh
    /// the shared store so the imported data appears immediately.
    private func importJournal(from url: URL) async {
        let needsScope = url.startAccessingSecurityScopedResource()
        defer {
            if needsScope { url.stopAccessingSecurityScopedResource() }
        }

        let source: String
        do {
            source = try String(contentsOf: url, encoding: .utf8)
        } catch {
            infoMessage = "Couldn't read \(url.lastPathComponent): \(error.localizedDescription)"
            return
        }

        do {
            let summary = try await client.importJournal(source: source)
            await store.reload(client: client)
            await syncManager.refreshStatus(client: client)
            let txnLabel = summary.transactionsImported == 1 ? "transaction" : "transactions"
            let acctLabel = summary.accountsCreated == 1 ? "account" : "accounts"
            infoMessage = """
            Imported \(url.lastPathComponent).

            \(summary.transactionsImported) \(txnLabel), \
            \(summary.accountsCreated) new \(acctLabel).
            """
        } catch {
            infoMessage = error.localizedDescription
        }
    }

    private var infoAlertBinding: Binding<Bool> {
        Binding(
            get: { infoMessage != nil },
            set: { if !$0 { infoMessage = nil } }
        )
    }
}
#endif
