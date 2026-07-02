import SwiftUI
import LdgrSwift

/// Full sync settings and status view.
///
/// Shows sync status, server connection settings, device ID, pending events,
/// conflicts, and a manual sync trigger. Onboarding is **discovery-driven**
/// (ADR-008): after connecting, the server's `/server/info` decides between the
/// two-secret (Secret Key + Emergency Kit) and legacy single-secret flows.
struct SyncSettingsView: View {
    let client: LdgrClient
    let syncManager: SyncManager

    // Server connection form (non-secret fields are persisted; secrets are not).
    @State private var baseURL = ""
    @State private var username = ""
    @State private var vaultId = ""
    @State private var password = ""
    @State private var secretKeyInput = ""

    // Discovery + flow state.
    @State private var serverInfo: ServerInfo?
    @State private var isConnecting = false
    @State private var isAuthenticating = false
    @State private var presentedKit: IdentifiableEmergencyKit?

    /// Whether this device already holds an account Secret Key (existing device).
    private var hasSecretKey: Bool { KeychainManager.hasSecretKey() }

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
        .sheet(item: $presentedKit) { wrapper in
            EmergencyKitView(kit: wrapper.kit) {
                presentedKit = nil
            }
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
                connectedServerView
            } else if let info = serverInfo {
                discoveredServerView(info)
            } else {
                connectView
            }
        }
    }

    @ViewBuilder
    private var connectedServerView: some View {
        LabeledContent("Server", value: baseURL)
        LabeledContent("Account", value: username)
        LabeledContent("Vault", value: vaultId)
        Button(role: .destructive) {
            signOut()
        } label: {
            Label("Sign Out", systemImage: "rectangle.portrait.and.arrow.right")
        }
    }

    /// Step 1 — enter the server URL and validate it via `/server/info`.
    @ViewBuilder
    private var connectView: some View {
        TextField("Server URL (https://…)", text: $baseURL)
            .textContentType(.URL)
            #if os(iOS)
            .keyboardType(.URL)
            .textInputAutocapitalization(.never)
            #endif
            .autocorrectionDisabled()

        Button {
            Task { await connect() }
        } label: {
            HStack {
                Label("Connect", systemImage: "network")
                Spacer()
                if isConnecting { ProgressView() }
            }
        }
        .disabled(baseURL.trimmingCharacters(in: .whitespaces).isEmpty || isConnecting)
    }

    /// Step 2 — the server told us its capabilities; branch on `twoSecretAuth`.
    @ViewBuilder
    private func discoveredServerView(_ info: ServerInfo) -> some View {
        LabeledContent("Server", value: info.name)
        LabeledContent("Protocol", value: "v\(info.protocolVersion)")
        LabeledContent("Two-Secret Auth") {
            Image(systemName: info.twoSecretAuth ? "checkmark.shield.fill" : "shield.slash")
                .foregroundStyle(info.twoSecretAuth ? .green : .secondary)
        }

        TextField("Username", text: $username)
            .textContentType(.username)
            #if os(iOS)
            .textInputAutocapitalization(.never)
            #endif
            .autocorrectionDisabled()
        SecureField("Password", text: $password)
            .textContentType(.password)
        TextField("Vault ID", text: $vaultId)
            #if os(iOS)
            .textInputAutocapitalization(.never)
            #endif
            .autocorrectionDisabled()

        if info.twoSecretAuth {
            twoSecretButtons(info)
        } else {
            singleSecretButtons
        }

        Button("Change Server") {
            serverInfo = nil
        }
        .font(.caption)
    }

    @ViewBuilder
    private func twoSecretButtons(_ info: ServerInfo) -> some View {
        // Existing device: the Secret Key is already in the Keychain, so
        // sign-in needs only the password. New device: prompt for the Secret Key.
        if hasSecretKey {
            authButton(title: "Sign In", icon: "person.crop.circle.badge.checkmark") {
                await signIn2skd(info: info, secretKey: nil)
            }
        } else {
            SecureField("Secret Key (from your Emergency Kit)", text: $secretKeyInput)
                #if os(iOS)
                .textInputAutocapitalization(.never)
                #endif
                .autocorrectionDisabled()
            authButton(
                title: "Sign In on This Device",
                icon: "person.crop.circle.badge.checkmark",
                enabled: !secretKeyInput.trimmingCharacters(in: .whitespaces).isEmpty
            ) {
                await signIn2skd(info: info, secretKey: secretKeyInput)
            }
        }

        authButton(title: "Create Account", icon: "person.crop.circle.badge.plus") {
            await signUp2skd(info: info)
        }
    }

    @ViewBuilder
    private var singleSecretButtons: some View {
        authButton(title: "Sign In", icon: "person.crop.circle.badge.checkmark") {
            await authenticateSingleSecret(register: false)
        }
        authButton(title: "Create Account & Sign In", icon: "person.crop.circle.badge.plus") {
            await authenticateSingleSecret(register: true)
        }
    }

    private func authButton(
        title: String,
        icon: String,
        enabled: Bool = true,
        action: @escaping () async -> Void
    ) -> some View {
        Button {
            Task { await action() }
        } label: {
            HStack {
                Label(title, systemImage: icon)
                Spacer()
                if isAuthenticating { ProgressView() }
            }
        }
        .disabled(!canSubmit || !enabled || isAuthenticating)
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
        !username.trimmingCharacters(in: .whitespaces).isEmpty
            && !vaultId.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
    }

    private func loadConfig() {
        let config = ServerConfigStore.load()
        baseURL = config.baseURL
        username = config.username
        vaultId = config.vaultId
    }

    private func trimmedURL() -> URL? {
        URL(string: baseURL.trimmingCharacters(in: .whitespaces))
    }

    // MARK: - Discovery

    /// Validate the server URL and fetch its capabilities.
    private func connect() async {
        guard let url = trimmedURL() else {
            syncManager.errorMessage = "Invalid server URL."
            return
        }
        isConnecting = true
        defer { isConnecting = false }
        do {
            let session = LdgrSync.makeSession(baseURL: url)
            serverInfo = try await session.serverInfo()
        } catch {
            syncManager.errorMessage = error.localizedDescription
        }
    }

    // MARK: - Two-secret auth (ADR-008)

    /// Create a new two-secret account: generate a Secret Key, register, sign
    /// in, persist the Secret Key, and present the Emergency Kit once.
    private func signUp2skd(info: ServerInfo) async {
        guard let url = trimmedURL() else {
            syncManager.errorMessage = "Invalid server URL."
            return
        }
        let user = username.trimmingCharacters(in: .whitespaces)
        let vault = vaultId.trimmingCharacters(in: .whitespaces)
        let passwordData = Data(password.utf8)

        isAuthenticating = true
        defer { isAuthenticating = false }

        do {
            let kdf = try client.kdfParams()
            let material = LdgrSync.generateSecretKey()
            let session = LdgrSync.makeSession(baseURL: url)

            _ = try await session.register2skd(
                username: user,
                accountId: material.accountId,
                password: passwordData,
                secretKey: material.secretKey,
                kdfParams: kdf
            )
            try await session.login2skd(
                username: user,
                password: passwordData,
                secretKey: material.secretKey,
                kdfParams: kdf
            )

            try await finishSignIn(session: session, user: user, vault: vault)
            try KeychainManager.storeSecretKey(material.secretKey)

            // Build + present the Emergency Kit (shown once). The address is the
            // sign-in server URL (baked into the QR payload for new-device recovery),
            // NOT the operator's display name.
            let kit = try LdgrSync.buildEmergencyKit(
                address: url.absoluteString,
                email: user,
                secretKey: material.secretKey,
                recoveryKey: nil
            )
            presentedKit = IdentifiableEmergencyKit(kit: kit)
        } catch {
            syncManager.errorMessage = error.localizedDescription
        }
    }

    /// Sign in with two-secret auth. `secretKey` is `nil` on an existing device
    /// (loaded from the Keychain) or provided when onboarding a new device.
    private func signIn2skd(info: ServerInfo, secretKey: String?) async {
        guard let url = trimmedURL() else {
            syncManager.errorMessage = "Invalid server URL."
            return
        }
        let user = username.trimmingCharacters(in: .whitespaces)
        let vault = vaultId.trimmingCharacters(in: .whitespaces)
        let passwordData = Data(password.utf8)

        let key = secretKey?.trimmingCharacters(in: .whitespaces)
            ?? KeychainManager.retrieveSecretKey()
        guard let key, !key.isEmpty else {
            syncManager.errorMessage =
                "Your account Secret Key is required to sign in on this device."
            return
        }

        isAuthenticating = true
        defer { isAuthenticating = false }

        do {
            let kdf = try client.kdfParams()
            let session = LdgrSync.makeSession(baseURL: url)
            try await session.login2skd(
                username: user,
                password: passwordData,
                secretKey: key,
                kdfParams: kdf
            )
            try await finishSignIn(session: session, user: user, vault: vault)
            try KeychainManager.storeSecretKey(key)
            secretKeyInput = ""
        } catch {
            syncManager.errorMessage = error.localizedDescription
        }
    }

    // MARK: - Single-secret auth (legacy fallback)

    private func authenticateSingleSecret(register: Bool) async {
        guard let url = trimmedURL() else {
            syncManager.errorMessage = "Invalid server URL."
            return
        }
        let user = username.trimmingCharacters(in: .whitespaces)
        let vault = vaultId.trimmingCharacters(in: .whitespaces)
        let passwordData = Data(password.utf8)

        isAuthenticating = true
        defer { isAuthenticating = false }

        do {
            let session = LdgrSync.makeSession(baseURL: url)
            if register {
                _ = try await session.register(username: user, password: passwordData)
            }
            try await session.login(username: user, password: passwordData)
            try await finishSignIn(session: session, user: user, vault: vault)
        } catch {
            syncManager.errorMessage = error.localizedDescription
        }
    }

    // MARK: - Shared sign-in tail

    /// Persist the session token + device id, enroll the vault, save the config,
    /// and (re)configure the sync manager. Shared by every auth path.
    private func finishSignIn(
        session: LdgrSyncSession,
        user: String,
        vault: String
    ) async throws {
        guard let token = await session.token() else {
            throw SyncSettingsError.noToken
        }

        // Best-effort enrollment: create the vault if it doesn't exist yet.
        _ = try? await session.createVault(vaultId: vault)

        let deviceId = try client.syncStatus().deviceId

        try KeychainManager.storeServerAuthToken(token)
        try KeychainManager.storeServerDeviceId(deviceId)
        ServerConfigStore.save(
            ServerConfig(baseURL: baseURL, username: user, vaultId: vault)
        )

        password = ""
        serverInfo = nil
        syncManager.configure(client: client)
        await syncManager.refreshStatus(client: client)
    }

    private func signOut() {
        try? KeychainManager.deleteServerAuthToken()
        try? KeychainManager.deleteServerDeviceId()
        // The Secret Key stays on the device: it is account-level, not
        // session-level, so the user can sign back in with just their password.
        password = ""
        serverInfo = nil
        syncManager.configure(client: client)
    }
}

/// Wraps an ``EmergencyKit`` so it can drive a `.sheet(item:)` presentation.
private struct IdentifiableEmergencyKit: Identifiable {
    let id = UUID()
    let kit: EmergencyKit
}

private enum SyncSettingsError: LocalizedError {
    case noToken

    var errorDescription: String? {
        switch self {
        case .noToken:
            return "Sign in succeeded but no session token was returned."
        }
    }
}
