import SwiftUI
import LdgrSwift

/// Vault creation flow: enter password, confirm, create vault, show recovery key.
struct VaultSetupView: View {
    @Bindable var appState: AppState
    @Binding var client: LdgrClient?

    @State private var vaultName = ""
    @State private var password = ""
    @State private var confirmPassword = ""
    @State private var isCreating = false
    @State private var showRecoveryKey = false
    @State private var recoveryKey = ""

    private var canCreate: Bool {
        !vaultName.isEmpty
            && password.count >= 8
            && password == confirmPassword
            && !isCreating
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(spacing: 16) {
                        Image(systemName: "lock.shield.fill")
                            .font(.system(size: 56))
                            .foregroundStyle(.tint)
                        Text("Create Your Vault")
                            .font(.title2.weight(.bold))
                        Text("Your financial data is encrypted with AES-256-GCM. Only you can decrypt it.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .listRowBackground(Color.clear)
                }

                Section("Vault Name") {
                    TextField("e.g. Personal Finances", text: $vaultName)
                        .textContentType(.name)
                        .autocorrectionDisabled()
                }

                Section("Master Password") {
                    SecureField("Password (8+ characters)", text: $password)
                        .textContentType(.newPassword)
                    SecureField("Confirm password", text: $confirmPassword)
                        .textContentType(.newPassword)

                    if !password.isEmpty && !confirmPassword.isEmpty && password != confirmPassword {
                        Label("Passwords don't match", systemImage: "xmark.circle")
                            .foregroundStyle(.red)
                            .font(.caption)
                    }
                }

                Section {
                    Button {
                        Task { await createVault() }
                    } label: {
                        HStack {
                            Spacer()
                            if isCreating {
                                ProgressView()
                                    .padding(.trailing, 4)
                                Text("Creating…")
                            } else {
                                Text("Create Vault")
                            }
                            Spacer()
                        }
                    }
                    .disabled(!canCreate)
                }
            }
            .navigationTitle("ldgr")
            .sheet(isPresented: $showRecoveryKey) {
                RecoveryKeyView(
                    recoveryKey: recoveryKey,
                    onDismiss: {
                        showRecoveryKey = false
                        appState.transitionToUnlocked()
                    }
                )
                .interactiveDismissDisabled()
            }
        }
    }

    private func createVault() async {
        isCreating = true
        defer { isCreating = false }

        do {
            // Re-initialize client in case directory changed
            let newClient = try LdgrClient(path: appState.vaultPath)
            client = newClient

            recoveryKey = try await newClient.createVault(password: password, name: vaultName)

            // Offer biometric enrollment
            if BiometricManager.availableType() != .none {
                if let key = try? newClient.exportSessionKey() {
                    try? KeychainManager.storeSessionKey(key)
                }
            }

            showRecoveryKey = true
        } catch {
            appState.setError(error.localizedDescription)
        }
    }
}
