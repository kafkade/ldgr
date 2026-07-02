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
    private static let serverAuthTokenAccount = "server-auth-token"
    private static let serverDeviceIdAccount = "server-device-id"
    private static let serverSecretKeyAccount = "server-secret-key"

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

    // MARK: - Server credentials (auth token + device id)
    //
    // Unlike the vault session key, these are NOT biometric-gated: background
    // sync must be able to read them without a user-presence prompt. They are
    // still device-only and only readable after first unlock. The user's server
    // password is NEVER persisted — only the session token derived from it.

    /// Store the server session token (replacing any existing value).
    static func storeServerAuthToken(_ token: String) throws {
        try storeServerValue(Data(token.utf8), account: serverAuthTokenAccount)
    }

    /// Retrieve the server session token, or `nil` if none is stored.
    static func retrieveServerAuthToken() -> String? {
        guard let data = retrieveServerValue(account: serverAuthTokenAccount) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Remove the stored server session token.
    static func deleteServerAuthToken() throws {
        try deleteServerValue(account: serverAuthTokenAccount)
    }

    /// Whether a server session token is stored.
    static func hasServerAuthToken() -> Bool {
        retrieveServerValue(account: serverAuthTokenAccount) != nil
    }

    /// Store the server-registered device id (replacing any existing value).
    static func storeServerDeviceId(_ deviceId: String) throws {
        try storeServerValue(Data(deviceId.utf8), account: serverDeviceIdAccount)
    }

    /// Retrieve the server-registered device id, or `nil` if none is stored.
    static func retrieveServerDeviceId() -> String? {
        guard let data = retrieveServerValue(account: serverDeviceIdAccount) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Remove the stored server device id.
    static func deleteServerDeviceId() throws {
        try deleteServerValue(account: serverDeviceIdAccount)
    }

    // MARK: - Account Secret Key (two-secret auth, ADR-008)
    //
    // The account Secret Key is the second factor for server auth. Like the
    // token/device-id, it is NOT biometric-gated so background sync can derive
    // `MK_auth` without a user-presence prompt; it is device-only and readable
    // only after first unlock. It is shown once (in the Emergency Kit) and never
    // leaves the device except as the SRP proof. The master password is never
    // stored — only the Secret Key, which is useless without it.

    /// Store the account Secret Key (replacing any existing value).
    static func storeSecretKey(_ secretKey: String) throws {
        try storeServerValue(Data(secretKey.utf8), account: serverSecretKeyAccount)
    }

    /// Retrieve the account Secret Key, or `nil` if none is stored.
    static func retrieveSecretKey() -> String? {
        guard let data = retrieveServerValue(account: serverSecretKeyAccount) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Remove the stored account Secret Key.
    static func deleteSecretKey() throws {
        try deleteServerValue(account: serverSecretKeyAccount)
    }

    /// Whether an account Secret Key is stored on this device.
    static func hasSecretKey() -> Bool {
        retrieveServerValue(account: serverSecretKeyAccount) != nil
    }

    // MARK: - Server credential helpers

    private static func storeServerValue(_ value: Data, account: String) throws {
        try? deleteServerValue(account: account)

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: value,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.storeFailed(status)
        }
    }

    private static func retrieveServerValue(account: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess else { return nil }
        return result as? Data
    }

    private static func deleteServerValue(account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]

        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError.deleteFailed(status)
        }
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
