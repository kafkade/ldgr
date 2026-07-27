/// Idiomatic Swift layer for ldgr's **server-sync** FFI surface (issue #200).
///
/// The heavy lifting (SRP-6a auth, endpoint routing, blob handling) lives in
/// `ldgr-core` and is exposed through the UniFFI-generated `LdgrSyncClient`,
/// whose methods are already `async`/`throws`. This file adds the two things a
/// host app needs around it:
///
///  1. ``URLSessionHTTPSender`` — the platform I/O seam. It conforms to the
///     generated `FfiHttpSender` callback protocol and performs the actual
///     network request over `URLSession`. This is the only place iOS touches
///     the network for server sync; the web build satisfies the same seam with
///     `fetch` in the separate `ldgr-wasm` crate.
///  2. ``LdgrSyncError`` — a Swift-native error mapped from the FFI
///     `FfiSyncError`, plus a convenience factory for building a client bound
///     to a base URL.
///
/// Usage:
/// ```swift
/// let client = LdgrSync.makeClient(baseURL: URL(string: "https://sync.example.com")!)
/// _ = try await client.register(username: "alice", password: Data("pw".utf8))
/// try await client.login(username: "alice", password: Data("pw".utf8))
/// try await client.createVault(vaultId: "vault-1")
/// // `ciphertext` is an encrypted batch blob produced by the vault crypto layer.
/// _ = try await client.putBatch(vaultId: "vault-1", deviceId: "dev-a",
///                               batchId: "batch-0001", ciphertext: ciphertext)
/// // Persist the token to resume later without re-authenticating:
/// let token = await client.token()
/// ```

import Foundation
import LdgrFFI

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

// MARK: - Error

/// Swift-native error for the server-sync surface, mapped from `FfiSyncError`.
public enum LdgrSyncError: Error, LocalizedError, Sendable {
    /// The transport failed (network, DNS, TLS, …). Retryable.
    case transport(String)
    /// The server returned a non-2xx status.
    case http(status: UInt16, message: String)
    /// A response body could not be decoded.
    case decode(String)
    /// The server's SRP proof (M2) did not verify.
    case proofMismatch
    /// An authenticated operation was attempted without a session token.
    case notAuthenticated
    /// SRP handshake failure.
    case srp(String)
    /// Invalid client input (bad UUID, malformed Secret Key, key derivation …).
    case invalidInput(String)

    public var errorDescription: String? {
        switch self {
        case .transport(let m): return "Transport error: \(m)"
        case .http(let status, let m): return "Server returned \(status): \(m)"
        case .decode(let m): return "Failed to decode response: \(m)"
        case .proofMismatch: return "Server authentication proof mismatch."
        case .notAuthenticated: return "Not authenticated — sign in first."
        case .srp(let m): return "SRP error: \(m)"
        case .invalidInput(let m): return "Invalid input: \(m)"
        }
    }

    public init(from ffi: FfiSyncError) {
        switch ffi {
        case .Transport(let message): self = .transport(message)
        case .Http(let status, let message): self = .http(status: status, message: message)
        case .Decode(let message): self = .decode(message)
        case .ProofMismatch: self = .proofMismatch
        case .NotAuthenticated: self = .notAuthenticated
        case .Srp(let message): self = .srp(message)
        case .InvalidInput(let message): self = .invalidInput(message)
        }
    }
}

// MARK: - URLSession transport seam

/// Executes ``FfiRawRequest``s over `URLSession` — the host I/O seam the core
/// `LdgrSyncClient` calls back into.
///
/// A non-2xx HTTP response is **not** thrown here; it is returned as an
/// ``FfiRawResponse`` so the core client can classify it. Only genuine
/// transport failures surface as `FfiSyncError.Transport`.
public final class URLSessionHTTPSender: FfiHttpSender, @unchecked Sendable {
    private let baseURL: URL
    private let session: URLSession

    /// - Parameters:
    ///   - baseURL: e.g. `https://sync.example.com`. A trailing slash is fine;
    ///     it composes with the absolute `/api/v1/...` paths the core produces.
    ///   - session: the `URLSession` to use (defaults to `.shared`).
    public init(baseURL: URL, session: URLSession = .shared) {
        // Drop any trailing slash so path concatenation is clean.
        if baseURL.absoluteString.hasSuffix("/") {
            self.baseURL = URL(string: String(baseURL.absoluteString.dropLast())) ?? baseURL
        } else {
            self.baseURL = baseURL
        }
        self.session = session
    }

    public func send(request: FfiRawRequest) async throws -> FfiRawResponse {
        guard var components = URLComponents(
            url: baseURL.appendingPathComponent(request.path.hasPrefix("/")
                ? String(request.path.dropFirst()) : request.path),
            resolvingAgainstBaseURL: false
        ) else {
            throw FfiSyncError.Transport(message: "could not build URL for path \(request.path)")
        }
        // appendingPathComponent percent-escapes the path; rebuild from the raw
        // base + path to preserve the server's exact `/api/v1/...` shape.
        components = URLComponents(string: baseURL.absoluteString + request.path)
            ?? components
        if !request.query.isEmpty {
            components.queryItems = request.query.map {
                URLQueryItem(name: $0.name, value: $0.value)
            }
        }
        guard let url = components.url else {
            throw FfiSyncError.Transport(message: "invalid URL for path \(request.path)")
        }

        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method.httpVerb
        for header in request.headers {
            urlRequest.setValue(header.value, forHTTPHeaderField: header.name)
        }
        if !request.body.isEmpty {
            urlRequest.httpBody = request.body
        }

        do {
            let (data, response) = try await session.data(for: urlRequest)
            guard let http = response as? HTTPURLResponse else {
                throw FfiSyncError.Transport(message: "non-HTTP response")
            }
            return FfiRawResponse(status: UInt16(truncatingIfNeeded: http.statusCode), body: data)
        } catch let error as FfiSyncError {
            throw error
        } catch {
            // Map every URLSession failure to a retryable transport error.
            throw FfiSyncError.Transport(message: error.localizedDescription)
        }
    }
}

private extension FfiHttpMethod {
    var httpVerb: String {
        switch self {
        case .get: return "GET"
        case .post: return "POST"
        case .put: return "PUT"
        case .delete: return "DELETE"
        }
    }
}

// MARK: - Convenience factory

/// Factory helpers for building a server-sync client bound to a base URL.
///
/// The returned value is the UniFFI-generated `LdgrSyncClient`, whose methods
/// are already `async`/`throws`. Catch ``FfiSyncError`` (or convert it via
/// ``LdgrSyncError/init(from:)``) at the call site.
public enum LdgrSync {
    /// Build a fresh, unauthenticated client targeting `baseURL`.
    public static func makeClient(
        baseURL: URL,
        session: URLSession = .shared
    ) -> LdgrSyncClient {
        LdgrSyncClient(sender: URLSessionHTTPSender(baseURL: baseURL, session: session))
    }

    /// Build a client that resumes a previously persisted session `token`.
    public static func makeClient(
        baseURL: URL,
        token: String,
        session: URLSession = .shared
    ) -> LdgrSyncClient {
        LdgrSyncClient.withToken(
            sender: URLSessionHTTPSender(baseURL: baseURL, session: session),
            token: token
        )
    }

    /// Build an idiomatic ``LdgrSyncSession`` targeting `baseURL`.
    ///
    /// Prefer this over ``makeClient(baseURL:session:)`` from host apps: the
    /// session exposes only Swift-native types and `LdgrSwift` errors, so callers
    /// don't need to import the generated `LdgrFFI` module.
    public static func makeSession(
        baseURL: URL,
        session: URLSession = .shared
    ) -> LdgrSyncSession {
        LdgrSyncSession(client: makeClient(baseURL: baseURL, session: session))
    }

    /// Build an idiomatic ``LdgrSyncSession`` that resumes a persisted `token`.
    public static func makeSession(
        baseURL: URL,
        token: String,
        session: URLSession = .shared
    ) -> LdgrSyncSession {
        LdgrSyncSession(client: makeClient(baseURL: baseURL, token: token, session: session))
    }

    // MARK: Two-secret onboarding helpers (ADR-008)

    /// Generate a fresh account Secret Key + account id for two-secret sign-up.
    ///
    /// Call this once when creating an account, render the returned
    /// ``SecretKeyMaterial/secretKey`` in the Emergency Kit, store it securely
    /// (Keychain), then pass it to ``LdgrSyncSession/register2skd(...)``.
    public static func generateSecretKey() -> SecretKeyMaterial {
        let m = LdgrFFI.generateSecretKey()
        return SecretKeyMaterial(
            accountId: m.accountId,
            secretKey: m.secretKey,
            accountHint: m.accountHint
        )
    }

    /// Generate a fresh **account-scoped** Argon2 KDF for two-secret sign-up
    /// (#296): a random salt paired with mobile Argon2 parameters. Decoupled
    /// from any vault, so it reproduces `MK_auth` on any device. Thread the same
    /// value through ``LdgrSyncSession/register2skd(...)``, ``buildEmergencyKit``,
    /// and secure storage (Keychain).
    public static func generateAccountKdf() -> KdfParams {
        KdfParams(
            salt: LdgrFFI.generateAccountKdfSalt(),
            memoryCostKib: 64 * 1024,
            iterations: 4,
            parallelism: 2
        )
    }

    /// Assemble the Emergency Kit shown once after a successful sign-up.
    ///
    /// - Parameters:
    ///   - address: the server address the account lives on.
    ///   - email: the account's email/username.
    ///   - secretKey: the account Secret Key from ``generateSecretKey()``.
    ///   - accountKdf: the account-scoped Argon2 KDF from ``generateAccountKdf()``.
    ///   - recoveryKey: optional vault recovery key to include.
    public static func buildEmergencyKit(
        address: String,
        email: String,
        secretKey: String,
        accountKdf: KdfParams,
        recoveryKey: String? = nil
    ) throws -> EmergencyKit {
        do {
            let k = try LdgrFFI.buildEmergencyKit(
                address: address,
                email: email,
                secretKey: secretKey,
                accountKdfSalt: accountKdf.salt,
                accountKdfParams: FfiArgon2Params(
                    memoryCostKib: accountKdf.memoryCostKib,
                    iterations: accountKdf.iterations,
                    parallelism: accountKdf.parallelism
                ),
                recoveryKey: recoveryKey
            )
            return EmergencyKit(from: k)
        } catch let error as FfiSyncError {
            throw LdgrSyncError(from: error)
        }
    }

    /// Parse a scanned/typed Emergency Kit QR payload back into its fields
    /// (#296), so a new device can extract the Secret Key + account KDF.
    public static func parseEmergencyKit(qrPayload: String) throws -> EmergencyKit {
        do {
            return EmergencyKit(from: try LdgrFFI.parseEmergencyKit(qrPayload: qrPayload))
        } catch let error as FfiSyncError {
            throw LdgrSyncError(from: error)
        }
    }
}

// MARK: - Idiomatic session wrapper

/// An account-scoped Argon2id salt and parameters for two-secret auth
/// (ADR-008 + #296).
///
/// `MK_auth` is derived from the master password with exactly these values.
/// Generated once at sign-up (``LdgrSync/generateAccountKdf()``), decoupled from
/// any vault, and carried in the Emergency Kit / returned by the server at
/// `login/init`. Pass it to the 2SKD session methods.
public struct KdfParams: Sendable {
    public let salt: Data
    public let memoryCostKib: UInt32
    public let iterations: UInt32
    public let parallelism: UInt32

    public init(salt: Data, memoryCostKib: UInt32, iterations: UInt32, parallelism: UInt32) {
        self.salt = salt
        self.memoryCostKib = memoryCostKib
        self.iterations = iterations
        self.parallelism = parallelism
    }
}

/// Discovery metadata from `GET /server/info` (ADR-008).
///
/// `twoSecretAuth` tells the client whether to run the two-secret onboarding
/// flow (Secret Key + Emergency Kit) or the legacy single-secret flow.
public struct ServerInfo: Sendable {
    public let name: String
    public let version: String
    public let protocolVersion: UInt32
    public let minProtocolVersion: UInt32
    public let maxProtocolVersion: UInt32
    public let registrationPolicy: String
    public let publicRegistration: Bool
    public let twoSecretAuth: Bool
}

/// Liveness/URL-validation probe result from `GET /server/ping`.
public struct ServerPong: Sendable {
    public let pong: Bool
    public let name: String
    public let protocolVersion: UInt32
}

/// A freshly generated account Secret Key plus the account id it is bound to
/// (ADR-008). Show `secretKey` once (in the Emergency Kit), store it securely,
/// and pass both to ``LdgrSyncSession/register2skd(...)``.
public struct SecretKeyMaterial: Sendable {
    /// Client-generated account id (UUID string) bound into the verifier.
    public let accountId: String
    /// Canonical `A1-…` Secret Key text. **Secret** — show once, store securely.
    public let secretKey: String
    /// Non-secret 6-char account-id hint (for pairing a kit to an account).
    public let accountHint: String
}

/// Render-agnostic Emergency Kit data for new-device sign-in (ADR-008 + #296).
public struct EmergencyKit: Sendable {
    public let version: UInt32
    public let address: String
    public let email: String
    public let accountHint: String
    /// Account Secret Key text. **Secret.**
    public let secretKey: String
    /// Account-scoped Argon2 KDF (salt + params) for deriving `MK_auth` on a new
    /// device (#296). Not secret.
    public let accountKdf: KdfParams
    /// Optional vault recovery key text (opt-in). **Secret.**
    public let recoveryKey: String?
    /// Versioned JSON payload the host renders into the kit's QR code.
    public let qrPayload: String

    init(from k: FfiEmergencyKit) {
        self.version = k.version
        self.address = k.address
        self.email = k.email
        self.accountHint = k.accountHint
        self.secretKey = k.secretKey
        self.accountKdf = KdfParams(
            salt: k.accountKdfSalt,
            memoryCostKib: k.accountKdfParams.memoryCostKib,
            iterations: k.accountKdfParams.iterations,
            parallelism: k.accountKdfParams.parallelism
        )
        self.recoveryKey = k.recoveryKey
        self.qrPayload = k.qrPayload
    }
}

/// Metadata about a remote encrypted batch, in Swift-native types.
public struct RemoteBatchMeta: Sendable {
    public let batchId: String
    public let deviceId: String
    public let path: String
    public let size: UInt64
    public let contentHash: String?
    /// Server-reported last-modified timestamp (RFC 3339). Used as the pull
    /// cursor high-water mark.
    public let modifiedAt: String?
}

/// Result of uploading an encrypted blob.
public struct PutBlobResult: Sendable {
    public let path: String
    public let size: UInt64
    public let contentHash: String
}

/// Idiomatic async/await wrapper around the server-sync FFI client.
///
/// Mirrors ``LdgrClient``: it hides the generated `LdgrFFI` types behind
/// Swift-native types (`Data`, `String`, ``RemoteBatchMeta``) and maps
/// `FfiSyncError` to ``LdgrSyncError``, so host apps only ever depend on the
/// `LdgrSwift` module. The heavy lifting (SRP-6a auth, routing) stays in Rust.
public final class LdgrSyncSession: @unchecked Sendable {
    private let client: LdgrSyncClient

    init(client: LdgrSyncClient) {
        self.client = client
    }

    /// The current session token, if authenticated.
    public func token() async -> String? {
        await client.token()
    }

    /// Whether the session holds a token.
    public func isAuthenticated() async -> Bool {
        await client.isAuthenticated()
    }

    /// Register a new single-secret account. Returns the new user id.
    public func register(username: String, password: Data) async throws -> String {
        try await mapping { try await client.register(username: username, password: password) }
    }

    /// Sign in (single-secret SRP-6a) and store the session token internally.
    public func login(username: String, password: Data) async throws {
        try await mapping { try await client.login(username: username, password: password) }
    }

    // MARK: Discovery

    /// Fetch server discovery metadata (`GET /server/info`).
    ///
    /// Use ``ServerInfo/twoSecretAuth`` to decide between two-secret and
    /// single-secret onboarding, and validate the protocol version.
    public func serverInfo() async throws -> ServerInfo {
        try await mapping {
            let i = try await client.serverInfo()
            return ServerInfo(
                name: i.name,
                version: i.version,
                protocolVersion: i.protocolVersion,
                minProtocolVersion: i.minProtocolVersion,
                maxProtocolVersion: i.maxProtocolVersion,
                registrationPolicy: i.registrationPolicy,
                publicRegistration: i.publicRegistration,
                twoSecretAuth: i.twoSecretAuth
            )
        }
    }

    /// Cheap liveness probe (`GET /server/ping`) for URL validation.
    public func ping() async throws -> ServerPong {
        try await mapping {
            let p = try await client.ping()
            return ServerPong(pong: p.pong, name: p.name, protocolVersion: p.protocolVersion)
        }
    }

    // MARK: Two-secret auth (2SKD, ADR-008)

    /// Register a new account using two-secret (2SKD) derivation.
    ///
    /// `MK_auth` is derived from `password` + the **account-scoped** Argon2 KDF
    /// (``LdgrSync/generateAccountKdf()``, decoupled from any vault, #296); the
    /// Secret Key comes from ``LdgrSync/generateSecretKey()``. Returns the new
    /// user id. Both secrets stay inside Rust.
    @discardableResult
    public func register2skd(
        username: String,
        accountId: String,
        password: Data,
        secretKey: String,
        accountKdf: KdfParams
    ) async throws -> String {
        try await mapping {
            try await client.register2skd(
                username: username,
                accountId: accountId,
                password: password,
                secretKey: secretKey,
                argon2Salt: accountKdf.salt,
                argon2Params: FfiArgon2Params(
                    memoryCostKib: accountKdf.memoryCostKib,
                    iterations: accountKdf.iterations,
                    parallelism: accountKdf.parallelism
                )
            )
        }
    }

    /// Perform a two-secret (2SKD) sign-in and store the session token.
    ///
    /// The account id is not required — the server returns it at `login/init`.
    /// On a new device, supply the master `password` and the account
    /// `secretKey` (typed or scanned from the Emergency Kit). `accountKdf` is
    /// optional: pass the Emergency-Kit copy to override the server's (#296), or
    /// `nil` to use the copy the server returns at `login/init`.
    public func login2skd(
        username: String,
        password: Data,
        secretKey: String,
        accountKdf: KdfParams? = nil
    ) async throws {
        try await mapping {
            try await client.login2skd(
                username: username,
                password: password,
                secretKey: secretKey,
                argon2Salt: accountKdf?.salt,
                argon2Params: accountKdf.map {
                    FfiArgon2Params(
                        memoryCostKib: $0.memoryCostKib,
                        iterations: $0.iterations,
                        parallelism: $0.parallelism
                    )
                }
            )
        }
    }

    /// Create a vault on the server (idempotent enrollment). Returns its path.
    @discardableResult
    public func createVault(vaultId: String) async throws -> String {
        try await mapping { try await client.createVault(vaultId: vaultId) }
    }

    /// Register/refresh this device's record on the server.
    public func putDevice(vaultId: String, deviceId: String, encryptedInfo: Data) async throws {
        try await mapping {
            try await client.putDevice(
                vaultId: vaultId,
                deviceId: deviceId,
                encryptedInfo: encryptedInfo
            )
        }
    }

    /// Upload an encrypted batch blob.
    @discardableResult
    public func putBatch(
        vaultId: String,
        deviceId: String,
        batchId: String,
        ciphertext: Data
    ) async throws -> PutBlobResult {
        try await mapping {
            let r = try await client.putBatch(
                vaultId: vaultId,
                deviceId: deviceId,
                batchId: batchId,
                ciphertext: ciphertext
            )
            return PutBlobResult(path: r.path, size: r.size, contentHash: r.contentHash)
        }
    }

    /// Download an encrypted batch blob.
    public func getBatch(vaultId: String, deviceId: String, batchId: String) async throws -> Data {
        try await mapping {
            try await client.getBatch(vaultId: vaultId, deviceId: deviceId, batchId: batchId)
        }
    }

    /// List remote batches, newest-first relative to `since` (RFC 3339 cursor).
    public func listRemoteBatches(
        vaultId: String,
        since: String?,
        deviceId: String?,
        limit: UInt32?
    ) async throws -> [RemoteBatchMeta] {
        try await mapping {
            try await client.listRemoteBatches(
                vaultId: vaultId,
                since: since,
                deviceId: deviceId,
                limit: limit
            ).map {
                RemoteBatchMeta(
                    batchId: $0.batchId,
                    deviceId: $0.deviceId,
                    path: $0.path,
                    size: $0.size,
                    contentHash: $0.contentHash,
                    modifiedAt: $0.modifiedAt
                )
            }
        }
    }

    /// Run `body`, converting any `FfiSyncError` into ``LdgrSyncError``.
    private func mapping<T>(_ body: () async throws -> T) async throws -> T {
        do {
            return try await body()
        } catch let error as FfiSyncError {
            throw LdgrSyncError(from: error)
        }
    }
}
