import SwiftUI
import LdgrSwift

/// Settings screen: biometrics toggle, auto-lock, lock vault.
struct SettingsView: View {
    @Bindable var appState: AppState
    let client: LdgrClient

    @Environment(\.dismiss) private var dismiss
    @State private var biometricEnabled: Bool = false

    var body: some View {
        NavigationStack {
            Form {
                // Security section
                Section("Security") {
                    let bioType = appState.biometricType
                    if bioType != .none {
                        Toggle(isOn: $biometricEnabled) {
                            Label(
                                BiometricManager.label(for: bioType),
                                systemImage: BiometricManager.systemImage(for: bioType)
                            )
                        }
                        .onChange(of: biometricEnabled) { _, newValue in
                            toggleBiometric(enabled: newValue)
                        }
                    }

                    Picker("Auto-Lock", selection: $appState.autoLockInterval) {
                        ForEach(AppState.AutoLockInterval.allCases) { interval in
                            Text(interval.label).tag(interval)
                        }
                    }
                }

                // Info section
                Section("Vault") {
                    if let name = try? client.vaultName() {
                        HStack {
                            Text("Name")
                            Spacer()
                            Text(name)
                                .foregroundStyle(.secondary)
                        }
                    }

                    if let accounts = try? client.listAccounts() {
                        HStack {
                            Text("Accounts")
                            Spacer()
                            Text("\(accounts.count)")
                                .foregroundStyle(.secondary)
                        }
                    }
                }

                // Actions
                Section {
                    Button(role: .destructive) {
                        client.close()
                        appState.transitionToLocked()
                        dismiss()
                    } label: {
                        Label("Lock Vault", systemImage: "lock")
                    }
                }
            }
            .navigationTitle("Settings")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .platformTrailing) {
                    Button("Done") { dismiss() }
                }
            }
            .onAppear {
                biometricEnabled = appState.isBiometricEnabled
            }
        }
    }

    // MARK: - Biometric Toggle

    private func toggleBiometric(enabled: Bool) {
        if enabled {
            // Store session key for biometric unlock
            guard let key = try? client.exportSessionKey() else {
                biometricEnabled = false
                appState.setError("Could not export session key. Please re-enter your password.")
                return
            }
            do {
                try KeychainManager.storeSessionKey(key)
            } catch {
                biometricEnabled = false
                appState.setError("Failed to enable biometrics: \(error.localizedDescription)")
            }
        } else {
            // Remove session key
            try? KeychainManager.deleteSessionKey()
        }
    }
}
