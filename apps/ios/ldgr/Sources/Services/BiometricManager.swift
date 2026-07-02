import Foundation
import LocalAuthentication

/// Checks biometric capability on the current device.
///
/// Used only for UI decisions (show/hide Face ID button). The actual
/// security gate is the Keychain access control, not LAContext.
enum BiometricManager {
    enum BiometricType: Sendable {
        case none
        case faceID
        case touchID
        /// A paired Apple Watch used for authorization (macOS only).
        case watch
    }

    /// Returns the available biometric type, or `.none` if unavailable.
    static func availableType() -> BiometricType {
        let context = LAContext()
        var error: NSError?

        if context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error) {
            switch context.biometryType {
            case .faceID:
                return .faceID
            case .touchID:
                return .touchID
            case .opticID:
                return .faceID // Treat Vision Pro optic ID like Face ID for UI
            default:
                break
            }
        }

        #if os(macOS)
        // On a Mac without Touch ID, a paired and unlocked Apple Watch can still
        // authorize the user. `.deviceOwnerAuthenticationWithWatch` is macOS-only.
        var watchError: NSError?
        if context.canEvaluatePolicy(.deviceOwnerAuthenticationWithWatch, error: &watchError) {
            return .watch
        }
        #endif

        return .none
    }

    /// Human-readable label for the biometric type.
    static func label(for type: BiometricType) -> String {
        switch type {
        case .none: return "Biometrics"
        case .faceID: return "Face ID"
        case .touchID: return "Touch ID"
        case .watch: return "Apple Watch"
        }
    }

    /// SF Symbol name for the biometric type.
    static func systemImage(for type: BiometricType) -> String {
        switch type {
        case .none: return "lock.shield"
        case .faceID: return "faceid"
        case .touchID: return "touchid"
        case .watch: return "applewatch"
        }
    }
}
