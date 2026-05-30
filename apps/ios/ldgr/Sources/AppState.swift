import Foundation
import SwiftUI

/// Central application state machine for vault lifecycle.
///
/// Drives the UI: which screen to show, whether biometrics are available,
/// and auto-lock behavior.
@Observable
final class AppState {
    // MARK: - Vault State

    enum VaultStatus: Sendable {
        /// No vault file exists — show setup screen.
        case noVault
        /// Vault exists but is locked — show unlock screen.
        case locked
        /// Currently unlocking (password or biometric in progress).
        case unlocking
        /// Vault is open and ready to use.
        case unlocked
    }

    private(set) var status: VaultStatus = .noVault

    /// The vault directory path (Documents/vault).
    let vaultPath: String

    /// Whether biometrics are available on this device.
    var biometricType: BiometricManager.BiometricType {
        BiometricManager.availableType()
    }

    /// Whether biometric unlock is enabled (session key stored in Keychain).
    var isBiometricEnabled: Bool {
        KeychainManager.hasSessionKey()
    }

    /// Last error message for display.
    var errorMessage: String?

    /// Recovery key from vault creation (shown once, then cleared).
    var pendingRecoveryKey: String?

    /// Timestamp when the app entered the background (for auto-lock).
    var backgroundTimestamp: Date?

    // MARK: - Settings

    enum AutoLockInterval: Double, CaseIterable, Identifiable {
        case immediate = 0
        case oneMinute = 60
        case fiveMinutes = 300
        case fifteenMinutes = 900

        var id: Double { rawValue }

        var label: String {
            switch self {
            case .immediate: return "Immediately"
            case .oneMinute: return "After 1 minute"
            case .fiveMinutes: return "After 5 minutes"
            case .fifteenMinutes: return "After 15 minutes"
            }
        }
    }

    var autoLockInterval: AutoLockInterval {
        get {
            let raw = UserDefaults.standard.double(forKey: "autoLockInterval")
            return AutoLockInterval(rawValue: raw) ?? .immediate
        }
        set {
            UserDefaults.standard.set(newValue.rawValue, forKey: "autoLockInterval")
        }
    }

    // MARK: - Init

    init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let vaultDir = docs.appendingPathComponent("vault")
        self.vaultPath = vaultDir.path

        // Check if vault exists
        let vaultFile = vaultDir.appendingPathComponent("vault.ldgr")
        if FileManager.default.fileExists(atPath: vaultFile.path) {
            self.status = .locked
        } else {
            self.status = .noVault
        }
    }

    // MARK: - State Transitions

    func transitionToUnlocked() {
        status = .unlocked
        errorMessage = nil
    }

    func transitionToLocked() {
        status = .locked
        errorMessage = nil
        pendingRecoveryKey = nil
    }

    func transitionToSetup() {
        status = .noVault
    }

    func transitionToUnlocking() {
        status = .unlocking
        errorMessage = nil
    }

    func setError(_ message: String) {
        errorMessage = message
        if status == .unlocking {
            status = .locked
        }
    }

    // MARK: - Auto-Lock

    /// Call when the app enters the background.
    func didEnterBackground() {
        backgroundTimestamp = Date()
    }

    /// Call when the app returns to the foreground.
    /// Returns `true` if the vault should be locked.
    func shouldLockOnForeground() -> Bool {
        guard status == .unlocked else { return false }

        let interval = autoLockInterval
        if interval == .immediate { return true }

        guard let bg = backgroundTimestamp else { return true }
        return Date().timeIntervalSince(bg) >= interval.rawValue
    }
}
