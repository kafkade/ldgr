import SwiftUI
import LdgrSwift

/// Adaptive navigation: TabView on iPhone, NavigationSplitView on iPad.
///
/// All tabs share a single `VaultDataStore` so mutations in one tab
/// are immediately visible in others.
struct MainTabView: View {
    @Bindable var appState: AppState
    let client: LdgrClient
    @State private var store = VaultDataStore()
    @State private var syncManager = SyncManager()
    @State private var selectedTab: Tab = .dashboard
    @State private var showSettings = false
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(WatchConnectivityManager.self) private var watchManager

    enum Tab: String, CaseIterable, Identifiable {
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
        Group {
            if sizeClass == .regular {
                iPadLayout
            } else {
                iPhoneLayout
            }
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                SyncSettingsView(client: client, syncManager: syncManager)
            }
        }
        .task {
            await store.reload(client: client)
            await syncManager.refreshStatus(client: client)
            await watchManager.sendUpdate(from: store, client: client)
        }
        .onChange(of: store.isLoading) { oldValue, newValue in
            if oldValue && !newValue {
                Task {
                    await watchManager.sendUpdate(from: store, client: client)
                }
            }
        }
    }

    // MARK: - iPhone Layout (Tab Bar)

    private var iPhoneLayout: some View {
        TabView(selection: $selectedTab) {
            ForEach(Tab.allCases) { tab in
                NavigationStack {
                    tabContent(for: tab)
                        .toolbar {
                            ToolbarItem(placement: .topBarLeading) {
                                lockButton
                            }
                            ToolbarItem(placement: .topBarTrailing) {
                                HStack(spacing: 12) {
                                    SyncStatusIndicator(syncManager: syncManager)
                                    settingsButton
                                }
                            }
                        }
                }
                .tabItem {
                    Label(tab.rawValue, systemImage: tab.icon)
                }
                .tag(tab)
            }
        }
    }

    // MARK: - iPad Layout (Sidebar + Detail)

    private var iPadLayout: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            NavigationStack {
                tabContent(for: selectedTab)
            }
        }
    }

    private var sidebar: some View {
        List(Tab.allCases, selection: $selectedTab) { tab in
            Label(tab.rawValue, systemImage: tab.icon)
                .tag(tab)
        }
        .navigationTitle(store.vaultName.isEmpty ? "ldgr" : store.vaultName)
        .toolbar {
            ToolbarItem(placement: .bottomBar) {
                HStack {
                    lockButton
                    Spacer()
                    SyncStatusIndicator(syncManager: syncManager)
                    settingsButton
                }
            }
        }
    }

    // MARK: - Tab Content

    @ViewBuilder
    private func tabContent(for tab: Tab) -> some View {
        switch tab {
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

    // MARK: - Shared Toolbar Buttons

    private var lockButton: some View {
        Button {
            client.close()
            appState.transitionToLocked()
        } label: {
            Image(systemName: "lock")
        }
    }

    private var settingsButton: some View {
        Button {
            showSettings = true
        } label: {
            Image(systemName: "gearshape")
        }
    }
}
