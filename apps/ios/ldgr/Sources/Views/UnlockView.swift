import SwiftUI
import LdgrSwift

/// Unlock screen with password entry and optional biometric button.
struct UnlockView: View {
    @Bindable var appState: AppState
    let client: LdgrClient

    @State private var password = ""
    @State private var isUnlocking = false
    @State private var showBiometricError = false
    @State private var biometricErrorMessage = ""

    private var canUnlock: Bool {
        !password.isEmpty && !isUnlocking
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 32) {
                Spacer()

                // App icon
                VStack(spacing: 12) {
                    Image(systemName: "lock.shield.fill")
                        .font(.system(size: 64))
                        .foregroundStyle(.tint)
                    Text("ldgr")
                        .font(.largeTitle.weight(.bold))
                    Text("Enter your master password")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                // Password field
                VStack(spacing: 12) {
                    SecureField("Master password", text: $password)
                        .textContentType(.password)
                        .submitLabel(.go)
                        .padding()
                        .background(.ultraThinMaterial)
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .onSubmit {
                            if canUnlock {
                                Task { await unlockWithPassword() }
                            }
                        }

                    Button {
                        Task { await unlockWithPassword() }
                    } label: {
                        HStack {
                            Spacer()
                            if isUnlocking && !appState.isBiometricEnabled {
                                ProgressView()
                                    .padding(.trailing, 4)
                                Text("Unlocking…")
                            } else {
                                Text("Unlock")
                            }
                            Spacer()
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!canUnlock)
                }
                .padding(.horizontal)

                // Biometric button
                if appState.isBiometricEnabled {
                    let bioType = appState.biometricType
                    Button {
                        Task { await unlockWithBiometrics() }
                    } label: {
                        Label(
                            BiometricManager.label(for: bioType),
                            systemImage: BiometricManager.systemImage(for: bioType)
                        )
                        .font(.title2)
                    }
                    .disabled(isUnlocking)
                }

                Spacer()
                Spacer()
            }
            .padding()
            .onAppear {
                // Auto-trigger biometrics if available
                if appState.isBiometricEnabled && appState.status == .locked {
                    Task {
                        // Small delay to let the UI settle
                        try? await Task.sleep(for: .milliseconds(300))
                        await unlockWithBiometrics()
                    }
                }
            }
            .alert("Biometric Unlock Unavailable", isPresented: $showBiometricError) {
                Button("OK") {}
            } message: {
                Text(biometricErrorMessage)
            }
        }
    }

    // MARK: - Password Unlock

    private func unlockWithPassword() async {
        isUnlocking = true
        appState.transitionToUnlocking()
        defer {
            isUnlocking = false
            password = ""
        }

        do {
            try await client.open(password: password)

            // Store session key for biometric unlock if biometrics are available
            if BiometricManager.availableType() != .none {
                if let key = try? client.exportSessionKey() {
                    try? KeychainManager.storeSessionKey(key)
                }
            }

            appState.transitionToUnlocked()
        } catch {
            appState.setError(error.localizedDescription)
        }
    }

    // MARK: - Biometric Unlock

    private func unlockWithBiometrics() async {
        isUnlocking = true
        appState.transitionToUnlocking()
        defer { isUnlocking = false }

        do {
            // Keychain read triggers biometric prompt
            let keyData = try KeychainManager.retrieveSessionKey()
            try await client.openWithSessionKey(keyData)
            appState.transitionToUnlocked()
        } catch let error as KeychainError {
            switch error {
            case .biometricFailed:
                // User cancelled — just go back to locked
                appState.transitionToLocked()
            case .notFound:
                // Biometric enrollment was invalidated (e.g., new fingerprint)
                biometricErrorMessage = "Biometric data has changed. Please unlock with your password to re-enable \(BiometricManager.label(for: appState.biometricType))."
                showBiometricError = true
                appState.transitionToLocked()
            default:
                appState.setError(error.localizedDescription)
            }
        } catch {
            appState.setError(error.localizedDescription)
        }
    }
}
