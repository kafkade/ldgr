import Foundation

/// Non-secret server-sync configuration, persisted in `UserDefaults`.
///
/// The user's server password is **never** stored here (or anywhere on disk) —
/// only the derived session token, which lives in the Keychain via
/// ``KeychainManager``. This struct holds the connection coordinates and the
/// per-vault pull cursor.
struct ServerConfig: Equatable, Sendable {
    var baseURL: String
    var username: String
    var vaultId: String

    static let empty = ServerConfig(baseURL: "", username: "", vaultId: "")

    /// Whether every field required to attempt a connection is present.
    var isComplete: Bool {
        !baseURL.trimmingCharacters(in: .whitespaces).isEmpty
            && !username.trimmingCharacters(in: .whitespaces).isEmpty
            && !vaultId.trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// The parsed base URL, if valid.
    var baseURLValue: URL? {
        let trimmed = baseURL.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }
        return URL(string: trimmed)
    }
}

/// `UserDefaults`-backed persistence for ``ServerConfig`` plus the per-vault
/// sync cursor (the `modifiedAt` high-water mark used to avoid re-downloading
/// remote batches).
enum ServerConfigStore {
    private enum Key {
        static let baseURL = "sync.server.baseURL"
        static let username = "sync.server.username"
        static let vaultId = "sync.server.vaultId"
        static func sinceCursor(vaultId: String) -> String { "sync.server.cursor.\(vaultId)" }
    }

    static func load(from defaults: UserDefaults = .standard) -> ServerConfig {
        ServerConfig(
            baseURL: defaults.string(forKey: Key.baseURL) ?? "",
            username: defaults.string(forKey: Key.username) ?? "",
            vaultId: defaults.string(forKey: Key.vaultId) ?? ""
        )
    }

    static func save(_ config: ServerConfig, to defaults: UserDefaults = .standard) {
        defaults.set(config.baseURL, forKey: Key.baseURL)
        defaults.set(config.username, forKey: Key.username)
        defaults.set(config.vaultId, forKey: Key.vaultId)
    }

    static func clear(from defaults: UserDefaults = .standard) {
        defaults.removeObject(forKey: Key.baseURL)
        defaults.removeObject(forKey: Key.username)
        defaults.removeObject(forKey: Key.vaultId)
    }

    static func sinceCursor(vaultId: String, from defaults: UserDefaults = .standard) -> String? {
        defaults.string(forKey: Key.sinceCursor(vaultId: vaultId))
    }

    static func setSinceCursor(
        _ value: String?,
        vaultId: String,
        to defaults: UserDefaults = .standard
    ) {
        let key = Key.sinceCursor(vaultId: vaultId)
        if let value {
            defaults.set(value, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }
}
