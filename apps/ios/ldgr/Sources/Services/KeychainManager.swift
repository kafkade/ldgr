import Foundation
import LocalAuthentication
import Security

/// Manages secure storage of the vault session key in the iOS Keychain.
///
/// The session key is stored with biometric access control — the Keychain
/// itself gates biometric authentication. We never call `LAContext.evaluatePolicy`
/// separately; instead, `SecItemCopyMatching` triggers the biometric prompt.
enum KeychainManager {
    private static let service = "com.kafkade.ldgr"
    private static let sessionKeyAccount = "vault-session-key"

    // MARK: - Store

    /// Store the session key with biometric protection.
    ///
    /// Uses `.biometryCurrentSet` so the item is invalidated if the user
    /// adds or removes a fingerprint/face — requiring a password re-entry.
    static func storeSessionKey(_ key: Data) throws {
        // Delete any existing item first
        try? deleteSessionKey()

        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil,
            kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            .biometryCurrentSet,
            &error
        ) else {
            throw KeychainError.accessControlFailed(
                error?.takeRetainedValue().localizedDescription ?? "unknown"
            )
        }

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: sessionKeyAccount,
            kSecValueData as String: key,
            kSecAttrAccessControl as String: access,
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.storeFailed(status)
        }
    }

    // MARK: - Retrieve (triggers biometric prompt)

    /// Retrieve the session key from the Keychain.
    ///
    /// This triggers the system biometric prompt if the item has biometric
    /// access control. The prompt text is set via the LAContext.
    static func retrieveSessionKey(prompt: String = "Unlock your vault") throws -> Data {
        let context = LAContext()
        context.localizedReason = prompt

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: sessionKeyAccount,
            kSecReturnData as String: true,
            kSecUseAuthenticationContext as String: context,
            kSecUseOperationPrompt as String: prompt,
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)

        switch status {
        case errSecSuccess:
            guard let data = result as? Data else {
                throw KeychainError.unexpectedData
            }
            return data
        case errSecItemNotFound:
            throw KeychainError.notFound
        case errSecAuthFailed, errSecUserCanceled:
            throw KeychainError.biometricFailed
        default:
            throw KeychainError.retrieveFailed(status)
        }
    }

    // MARK: - Delete

    /// Remove the session key from the Keychain.
    static func deleteSessionKey() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: sessionKeyAccount,
        ]

        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError.deleteFailed(status)
        }
    }

    // MARK: - Query

    /// Check whether a session key exists in the Keychain (without authenticating).
    static func hasSessionKey() -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: sessionKeyAccount,
            kSecUseAuthenticationUI as String: kSecUseAuthenticationUIFail,
        ]

        let status = SecItemCopyMatching(query as CFDictionary, nil)
        // errSecInteractionNotAllowed means item exists but requires auth
        return status == errSecSuccess || status == errSecInteractionNotAllowed
    }
}

// MARK: - Errors

enum KeychainError: LocalizedError {
    case accessControlFailed(String)
    case storeFailed(OSStatus)
    case retrieveFailed(OSStatus)
    case deleteFailed(OSStatus)
    case notFound
    case biometricFailed
    case unexpectedData

    var errorDescription: String? {
        switch self {
        case .accessControlFailed(let msg):
            return "Failed to create access control: \(msg)"
        case .storeFailed(let status):
            return "Failed to store in Keychain (status: \(status))"
        case .retrieveFailed(let status):
            return "Failed to retrieve from Keychain (status: \(status))"
        case .deleteFailed(let status):
            return "Failed to delete from Keychain (status: \(status))"
        case .notFound:
            return "No session key found in Keychain"
        case .biometricFailed:
            return "Biometric authentication failed or was cancelled"
        case .unexpectedData:
            return "Unexpected data format in Keychain"
        }
    }
}
