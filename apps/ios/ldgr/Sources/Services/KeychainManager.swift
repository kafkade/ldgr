import Foundation
import LocalAuthentication
import Security

/// Manages secure storage of the vault session key in the Apple Keychain.
///
/// The session key is stored with biometric / user-presence access control —
/// the Keychain itself gates authentication. We never call
/// `LAContext.evaluatePolicy` separately; instead, `SecItemCopyMatching`
/// triggers the biometric (Touch ID / Apple Watch) or password prompt.
///
/// On macOS every query opts into the **data protection keychain** so that
/// generic-password items behave exactly like on iOS. See `baseQuery()`.
enum KeychainManager {
    private static let service = "com.kafkade.ldgr"
    private static let sessionKeyAccount = "vault-session-key"
    private static let serverAuthTokenAccount = "server-auth-token"
    private static let serverDeviceIdAccount = "server-device-id"
    private static let serverSecretKeyAccount = "server-secret-key"

    // MARK: - Query base

    /// Attributes shared by every Keychain query.
    ///
    /// On macOS this opts into the **data protection keychain**
    /// (`kSecUseDataProtectionKeychain`) so that generic-password items honor
    /// `kSecAttrAccessible…ThisDeviceOnly` accessibility, biometric /
    /// user-presence access control, and app-group scoping — exactly like iOS.
    /// The legacy file-based macOS keychain silently ignores several of these
    /// attributes. Using the data protection keychain requires the app-group
    /// (or keychain-access-group) entitlement, which the sandboxed macOS target
    /// has (`group.com.kafkade.ldgr`).
    ///
    /// It must be applied consistently to store, retrieve, and delete queries;
    /// otherwise items written to one keychain can't be found in the other.
    private static func baseQuery(account: String) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        #if os(macOS)
        query[kSecUseDataProtectionKeychain as String] = true
        #endif
        return query
    }

    /// Access-control flags gating the biometric-protected session key.
    ///
    /// - iOS: `.biometryCurrentSet` — requires the currently-enrolled biometric
    ///   set; adding or removing a fingerprint/face invalidates the item and
    ///   forces a password re-entry.
    /// - macOS: `.userPresence` — allows Touch ID, a paired Apple Watch, or the
    ///   account password as fallback, matching native macOS unlock UX. (macOS
    ///   does not support `.biometryCurrentSet`-style invalidation with the
    ///   watch/password fallbacks the platform expects.)
    private static var sessionKeyAccessControlFlags: SecAccessControlCreateFlags {
        #if os(macOS)
        return .userPresence
        #else
        return .biometryCurrentSet
        #endif
    }

    // MARK: - Store

    /// Store the session key with biometric protection.
    ///
    /// On iOS uses `.biometryCurrentSet` so the item is invalidated if the user
    /// adds or removes a fingerprint/face — requiring a password re-entry. On
    /// macOS uses `.userPresence` (Touch ID / Apple Watch / password fallback).
    static func storeSessionKey(_ key: Data) throws {
        // Delete any existing item first
        try? deleteSessionKey()

        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil,
            kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            sessionKeyAccessControlFlags,
            &error
        ) else {
            throw KeychainError.accessControlFailed(
                error?.takeRetainedValue().localizedDescription ?? "unknown"
            )
        }

        var query = baseQuery(account: sessionKeyAccount)
        query[kSecValueData as String] = key
        query[kSecAttrAccessControl as String] = access

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

        var query = baseQuery(account: sessionKeyAccount)
        query[kSecReturnData as String] = true
        query[kSecUseAuthenticationContext as String] = context

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
        let query = baseQuery(account: sessionKeyAccount)

        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError.deleteFailed(status)
        }
    }

    // MARK: - Query

    /// Check whether a session key exists in the Keychain (without authenticating).
    static func hasSessionKey() -> Bool {
        // A non-interactive context: the lookup must never prompt the user.
        let context = LAContext()
        context.interactionNotAllowed = true

        var query = baseQuery(account: sessionKeyAccount)
        query[kSecUseAuthenticationContext as String] = context

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

        var query = baseQuery(account: account)
        query[kSecValueData as String] = value
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError.storeFailed(status)
        }
    }

    private static func retrieveServerValue(account: String) -> Data? {
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess else { return nil }
        return result as? Data
    }

    private static func deleteServerValue(account: String) throws {
        let query = baseQuery(account: account)

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
