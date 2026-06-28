import SwiftUI
import LdgrSwift

/// Full sync settings and status view.
///
/// Shows sync status, server connection settings (sign in / out), device ID,
/// pending events, conflicts, and a manual sync trigger.
struct SyncSettingsView: View {
    let client: LdgrClient
    let syncManager: SyncManager

    // Server connection form (non-secret fields are persisted; password is not).
    @State private var baseURL = ""
    @State private var username = ""
    @State private var vaultId = ""
    @State private var password = ""
    @State private var isAuthenticated = false
    @State private var isAuthenticating = false

    var body: some View {
        List {
            statusSection
            actionsSection
            serverSection

            if !syncManager.conflicts.isEmpty {
                conflictsSection
            }

            deviceSection
        }
        .navigationTitle("Sync")
        .task {
            loadConfig()
            syncManager.configure(client: client)
            await syncManager.refreshStatus(client: client)
        }
        .refreshable {
            await syncManager.refreshStatus(client: client)
        }
        .alert(
            "Sync Error",
            isPresented: Binding(
                get: { syncManager.errorMessage != nil },
                set: { if !$0 { syncManager.errorMessage = nil } }
            )
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(syncManager.errorMessage ?? "")
        }
    }

    // MARK: - Sections

    private var statusSection: some View {
        Section("Status") {
            if let status = syncManager.status {
                LabeledContent("Pending Changes") {
                    Text("\(status.pendingEventCount)")
                        .foregroundStyle(status.pendingEventCount > 0 ? .blue : .secondary)
                }

                LabeledContent("Unresolved Conflicts") {
                    Text("\(status.unresolvedConflictCount)")
                        .foregroundStyle(status.unresolvedConflictCount > 0 ? .orange : .secondary)
                }

                if let lastSync = status.lastSyncAt {
                    LabeledContent("Last Sync") {
                        Text(String(lastSync.prefix(19)))
                            .font(.caption)
                    }
                } else {
                    LabeledContent("Last Sync") {
                        Text("Never")
                            .foregroundStyle(.secondary)
                    }
                }
            } else {
                HStack {
                    ProgressView()
                    Text("Loading…")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var actionsSection: some View {
        Section {
            Button {
                Task { await syncManager.sync(client: client) }
            } label: {
                HStack {
                    Label("Sync Now", systemImage: "arrow.triangle.2.circlepath")
                    Spacer()
                    if syncManager.isSyncing {
                        ProgressView()
                    }
                }
            }
            .disabled(syncManager.isSyncing || !syncManager.isServerConfigured)

            if !syncManager.isServerConfigured {
                Text("Sign in to a sync server below to enable syncing.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var serverSection: some View {
        Section("Sync Server") {
            if syncManager.isServerConfigured {
                LabeledContent("Server", value: baseURL)
                LabeledContent("Account", value: username)
                LabeledContent("Vault", value: vaultId)
                Button(role: .destructive) {
                    signOut()
                } label: {
                    Label("Sign Out", systemImage: "rectangle.portrait.and.arrow.right")
                }
            } else {
                TextField("Server URL (https://…)", text: $baseURL)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("Username", text: $username)
                    .textContentType(.username)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField("Password", text: $password)
                    .textContentType(.password)
                TextField("Vault ID", text: $vaultId)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()

                Button {
                    Task { await authenticate(register: false) }
                } label: {
                    HStack {
                        Label("Sign In", systemImage: "person.crop.circle.badge.checkmark")
                        Spacer()
                        if isAuthenticating { ProgressView() }
                    }
                }
                .disabled(!canSubmit || isAuthenticating)

                Button {
                    Task { await authenticate(register: true) }
                } label: {
                    Label("Create Account & Sign In", systemImage: "person.crop.circle.badge.plus")
                }
                .disabled(!canSubmit || isAuthenticating)
            }
        }
    }

    private var conflictsSection: some View {
        Section("Conflicts (\(syncManager.conflicts.count))") {
            NavigationLink {
                ConflictListView(
                    client: client,
                    syncManager: syncManager
                )
            } label: {
                Label(
                    "\(syncManager.conflicts.count) conflict\(syncManager.conflicts.count == 1 ? "" : "s") to review",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
            }
        }
    }

    private var deviceSection: some View {
        Section("Device") {
            if let status = syncManager.status {
                LabeledContent("Device ID") {
                    Text(String(status.deviceId.prefix(8)) + "…")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    // MARK: - Form helpers

    private var canSubmit: Bool {
        !baseURL.trimmingCharacters(in: .whitespaces).isEmpty
            && !username.trimmingCharacters(in: .whitespaces).isEmpty
            && !vaultId.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
            && URL(string: baseURL.trimmingCharacters(in: .whitespaces)) != nil
    }

    private func loadConfig() {
        let config = ServerConfigStore.load()
        baseURL = config.baseURL
        username = config.username
        vaultId = config.vaultId
    }

    /// Authenticate against the server, persist the token + device id, and
    /// (re)configure the sync manager. When `register` is true, create the
    /// account first.
    private func authenticate(register: Bool) async {
        guard let url = URL(string: baseURL.trimmingCharacters(in: .whitespaces)) else {
            syncManager.errorMessage = "Invalid server URL."
            return
        }
        let trimmedUser = username.trimmingCharacters(in: .whitespaces)
        let trimmedVault = vaultId.trimmingCharacters(in: .whitespaces)
        let passwordData = Data(password.utf8)

        isAuthenticating = true
        defer { isAuthenticating = false }

        do {
            let session = LdgrSync.makeSession(baseURL: url)
            if register {
                _ = try await session.register(username: trimmedUser, password: passwordData)
            }
            try await session.login(username: trimmedUser, password: passwordData)

            guard let token = await session.token() else {
                syncManager.errorMessage = "Sign in succeeded but no session token was returned."
                return
            }

            // Best-effort enrollment: create the vault if it doesn't exist yet.
            _ = try? await session.createVault(vaultId: trimmedVault)

            let deviceId = try client.syncStatus().deviceId

            try KeychainManager.storeServerAuthToken(token)
            try KeychainManager.storeServerDeviceId(deviceId)
            ServerConfigStore.save(
                ServerConfig(baseURL: baseURL, username: trimmedUser, vaultId: trimmedVault)
            )

            password = ""
            syncManager.configure(client: client)
            await syncManager.refreshStatus(client: client)
        } catch {
            syncManager.errorMessage = error.localizedDescription
        }
    }

    private func signOut() {
        try? KeychainManager.deleteServerAuthToken()
        try? KeychainManager.deleteServerDeviceId()
        password = ""
        syncManager.configure(client: client)
    }
}
