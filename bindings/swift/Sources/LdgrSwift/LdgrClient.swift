/// Idiomatic Swift async/await wrapper for ldgr FFI bindings.
///
/// Wraps the synchronous UniFFI-generated `LdgrVault` in Swift concurrency,
/// dispatching heavy crypto operations (vault creation, unlock) to a background
/// executor so they never block the main actor.
///
/// Usage:
/// ```swift
/// let client = try LdgrClient(path: FileManager.documentsPath)
/// let recoveryKey = try await client.createVault(password: "secret", name: "My Finances")
/// // IMPORTANT: Show recoveryKey to the user and have them save it!
///
/// let accountId = try client.addAccount(name: "Assets:Checking", type: .asset, commodity: "USD")
/// let txId = try client.addTransaction(
///     date: "2024-06-15",
///     description: "Grocery store",
///     status: .cleared,
///     postings: [
///         .init(accountId: accountId, amount: "-50.00", commodity: "USD"),
///         .init(accountId: expenseId, amount: "50.00", commodity: "USD"),
///     ]
/// )
/// let balances = try client.balance()
/// ```

import Foundation
import LdgrFFI

// MARK: - Public Types

/// Account type classification.
public enum AccountKind: String, Sendable {
    case asset
    case liability
    case income
    case expense
    case equity
}

/// Transaction clearing status.
public enum TransactionKind: String, Sendable {
    case unmarked
    case pending
    case cleared
}

/// A posting to include when creating a new transaction.
public struct NewPosting: Sendable {
    public let accountId: String
    public let amount: String?
    public let commodity: String?

    public init(accountId: String, amount: String? = nil, commodity: String? = nil) {
        self.accountId = accountId
        self.amount = amount
        self.commodity = commodity
    }
}

/// An account in the vault.
public struct Account: Sendable, Identifiable {
    public let id: String
    public let name: String
    public let type: AccountKind
    public let commodity: String?
}

/// A transaction with its postings.
public struct Transaction: Sendable, Identifiable {
    public let id: String
    public let date: String
    public let description: String
    public let status: TransactionKind
    public let postings: [Posting]
}

/// A posting within a transaction.
public struct Posting: Sendable, Identifiable {
    public let id: String
    public let accountId: String
    public let amount: String?
    public let commodity: String?
}

/// A single balance entry (account + amount + commodity).
public struct BalanceEntry: Sendable {
    public let account: String
    public let amount: String
    public let commodity: String
}

/// A pending sync event in the outbox.
public struct SyncEvent: Sendable, Identifiable {
    public let id: String
    public let deviceId: String
    public let entityType: String
    public let entityId: String
    public let operation: String
    public let lamportClock: UInt64
    public let synced: Bool
}

/// A sync conflict requiring user resolution.
public struct SyncConflict: Sendable, Identifiable {
    public let id: String
    public let entityType: String
    public let entityId: String
    public let localPayload: String
    public let remotePayload: String
    public let detectedAt: String
}

/// Resolution strategy for a sync conflict.
public enum ConflictResolution: String, Sendable {
    case keepLocal = "keep_local"
    case keepRemote = "keep_remote"
}

/// Current sync status summary.
public struct SyncStatus: Sendable {
    public let pendingEventCount: UInt64
    public let unresolvedConflictCount: UInt64
    public let lastSyncAt: String?
    public let deviceId: String
}

/// A composed, encrypted batch blob ready to upload to the sync server.
///
/// Produced by ``LdgrClient/exportPendingBatch()``. After a *successful*
/// upload, mark the included events synced with
/// ``LdgrClient/markEventsSynced(eventIds:)`` using ``eventIds``.
public struct ExportedBatch: Sendable {
    /// Random id for the blob (suitable as the `{batch}.enc` filename).
    public let batchId: String
    /// The device that produced the batch.
    public let deviceId: String
    /// The canonical encrypted blob bytes to upload via `putBatch`.
    public let ciphertext: Data
    /// Ids of the outbox events included — mark these synced after upload.
    public let eventIds: [String]
}

/// Outcome of applying a downloaded batch blob via ``LdgrClient/ingestBatch(ciphertext:)``.
public struct IngestOutcome: Sendable {
    /// Events applied cleanly to the canonical tables.
    public let applied: UInt32
    /// Conflicts detected and persisted for user review (see `listConflicts`).
    public let conflicts: UInt32
    /// Events skipped as already-seen or stale (no-op).
    public let skipped: UInt32
}

/// Errors from the ldgr vault.
public enum LdgrClientError: Error, LocalizedError, Sendable {
    case vaultLocked
    case invalidPassword
    case cryptoError(String)
    case storageError(String)
    case invalidInput(String)
    case notFound(String)
    case conflict(String)
    case ioError(String)

    public var errorDescription: String? {
        switch self {
        case .vaultLocked: return "The vault is locked. Please unlock it first."
        case .invalidPassword: return "Incorrect password."
        case .cryptoError(let msg): return "Crypto error: \(msg)"
        case .storageError(let msg): return "Storage error: \(msg)"
        case .invalidInput(let msg): return "Invalid input: \(msg)"
        case .notFound(let msg): return "Not found: \(msg)"
        case .conflict(let msg): return "Conflict: \(msg)"
        case .ioError(let msg): return "I/O error: \(msg)"
        }
    }

    init(from ffiError: LdgrError) {
        switch ffiError {
        case .VaultLocked:
            self = .vaultLocked
        case .InvalidPassword:
            self = .invalidPassword
        case .CryptoError:
            self = .cryptoError(String(describing: ffiError))
        case .StorageError:
            self = .storageError(String(describing: ffiError))
        case .InvalidInput:
            self = .invalidInput(String(describing: ffiError))
        case .NotFound:
            self = .notFound(String(describing: ffiError))
        case .Conflict:
            self = .conflict(String(describing: ffiError))
        case .IoError:
            self = .ioError(String(describing: ffiError))
        }
    }
}

// MARK: - LdgrClient

/// Thread-safe async wrapper around the ldgr vault.
///
/// Heavy operations (create, open) are dispatched to a background executor.
/// Light operations (list, balance) run synchronously but are still safe
/// to call from any actor context.
public final class LdgrClient: @unchecked Sendable {
    private let vault: LdgrVault

    /// Create a client pointing at the given vault directory.
    ///
    /// Does not open or create anything — call `createVault` or `open` next.
    public init(path: String) throws {
        do {
            self.vault = try LdgrVault(path: path)
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Whether the vault is currently unlocked.
    public var isUnlocked: Bool {
        vault.isUnlocked()
    }

    // MARK: - Heavy Operations (async)

    /// Create a new vault. Returns the recovery key.
    ///
    /// This is computationally expensive (Argon2 key derivation) and runs
    /// on a background thread.
    ///
    /// - Important: Present the recovery key to the user immediately.
    ///   It cannot be retrieved later.
    public func createVault(password: String, name: String) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            Task.detached { [vault] in
                do {
                    let key = try vault.createVault(password: password, name: name)
                    continuation.resume(returning: key)
                } catch let error as LdgrError {
                    continuation.resume(throwing: LdgrClientError(from: error))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    /// Unlock an existing vault with the given password.
    ///
    /// This is computationally expensive (Argon2 key derivation) and runs
    /// on a background thread.
    public func open(password: String) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            Task.detached { [vault] in
                do {
                    try vault.open(password: password)
                    continuation.resume()
                } catch let error as LdgrError {
                    continuation.resume(throwing: LdgrClientError(from: error))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    /// Lock the vault, clearing all in-memory keys.
    public func close() {
        vault.close()
    }

    /// Export the session key for secure caching (e.g., iOS Keychain).
    ///
    /// Returns exactly 32 bytes. These bytes grant full vault access —
    /// store them only in a biometric-protected Keychain item.
    public func exportSessionKey() throws -> Data {
        do {
            let bytes = try vault.exportSessionKey()
            return Data(bytes)
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// The vault's Argon2id salt and parameters, for two-secret (2SKD) sign-in.
    ///
    /// Two-secret auth derives the server auth key `MK_auth` from the master
    /// password using exactly these values (ADR-008). Requires the vault to be
    /// unlocked. Pass the result to ``LdgrSyncSession/register2skd(...)`` or
    /// ``LdgrSyncSession/login2skd(...)``.
    public func kdfParams() throws -> KdfParams {
        do {
            let p = try vault.kdfParams()
            return KdfParams(
                salt: Data(p.salt),
                memoryCostKib: p.memoryCostKib,
                iterations: p.iterations,
                parallelism: p.parallelism
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Unlock the vault using a previously exported session key.
    ///
    /// Skips Argon2id derivation — used for biometric unlock where the
    /// session key was stored in the OS keychain.
    public func openWithSessionKey(_ key: Data) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            Task.detached { [vault] in
                do {
                    try vault.openWithSessionKey(key: Array(key))
                    continuation.resume()
                } catch let error as LdgrError {
                    continuation.resume(throwing: LdgrClientError(from: error))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    // MARK: - Light Operations (sync, still safe from any actor)

    /// Get the vault name.
    public func vaultName() throws -> String {
        do {
            return try vault.vaultName()
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Create a new account. Returns the account ID.
    public func addAccount(
        name: String,
        type: AccountKind,
        commodity: String? = nil
    ) throws -> String {
        do {
            return try vault.addAccount(
                name: name,
                accountType: type.rawValue,
                commodity: commodity
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// List all accounts.
    public func listAccounts() throws -> [Account] {
        do {
            return try vault.listAccounts().map { ffi in
                Account(
                    id: ffi.id,
                    name: ffi.name,
                    type: AccountKind(rawValue: ffi.accountType) ?? .asset,
                    commodity: ffi.commodity
                )
            }
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Add a new transaction. Returns the transaction ID.
    public func addTransaction(
        date: String,
        description: String,
        status: TransactionKind = .unmarked,
        postings: [NewPosting]
    ) throws -> String {
        do {
            let ffiPostings = postings.map { p in
                FfiNewPosting(
                    accountId: p.accountId,
                    amount: p.amount,
                    commodity: p.commodity
                )
            }
            return try vault.addTransaction(
                date: date,
                description: description,
                status: status.rawValue,
                postings: ffiPostings
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// List all transactions.
    public func listTransactions() throws -> [Transaction] {
        do {
            return try vault.listTransactions().map { ffi in
                Transaction(
                    id: ffi.id,
                    date: ffi.date,
                    description: ffi.description,
                    status: TransactionKind(rawValue: ffi.status) ?? .unmarked,
                    postings: ffi.postings.map { p in
                        Posting(
                            id: p.id,
                            accountId: p.accountId,
                            amount: p.amount,
                            commodity: p.commodity
                        )
                    }
                )
            }
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Soft-delete a transaction.
    public func deleteTransaction(id: String) throws {
        do {
            try vault.deleteTransaction(id: id)
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Compute account balances.
    public func balance(
        accountFilter: String? = nil,
        beginDate: String? = nil,
        endDate: String? = nil
    ) throws -> [BalanceEntry] {
        do {
            return try vault.balance(
                accountFilter: accountFilter,
                beginDate: beginDate,
                endDate: endDate
            ).map { ffi in
                BalanceEntry(
                    account: ffi.account,
                    amount: ffi.amount,
                    commodity: ffi.commodity
                )
            }
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    // MARK: - Sync Operations

    /// Get current sync status.
    public func syncStatus() throws -> SyncStatus {
        do {
            let ffi = try vault.syncStatus()
            return SyncStatus(
                pendingEventCount: ffi.pendingEventCount,
                unresolvedConflictCount: ffi.unresolvedConflictCount,
                lastSyncAt: ffi.lastSyncAt,
                deviceId: ffi.deviceId
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Get all pending (un-synced) events.
    public func pendingSyncEvents() throws -> [SyncEvent] {
        do {
            return try vault.pendingSyncEvents().map { ffi in
                SyncEvent(
                    id: ffi.id,
                    deviceId: ffi.deviceId,
                    entityType: ffi.entityType,
                    entityId: ffi.entityId,
                    operation: ffi.operation,
                    lamportClock: ffi.lamportClock,
                    synced: ffi.synced
                )
            }
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Mark events as synced after successful push.
    public func markEventsSynced(eventIds: [String]) throws {
        do {
            try vault.markEventsSynced(eventIds: eventIds)
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Get all unresolved sync conflicts.
    public func listConflicts() throws -> [SyncConflict] {
        do {
            return try vault.listConflicts().map { ffi in
                SyncConflict(
                    id: ffi.id,
                    entityType: ffi.entityType,
                    entityId: ffi.entityId,
                    localPayload: ffi.localPayload,
                    remotePayload: ffi.remotePayload,
                    detectedAt: ffi.detectedAt
                )
            }
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Resolve a sync conflict.
    public func resolveConflict(id: String, resolution: ConflictResolution) throws {
        do {
            try vault.resolveConflict(conflictId: id, resolution: resolution.rawValue)
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Compose all currently-pending sync events into one encrypted batch blob.
    ///
    /// Returns `nil` when there are no pending events. Does **not** mark events
    /// synced — upload the ``ExportedBatch/ciphertext`` first, then call
    /// ``markEventsSynced(eventIds:)`` with ``ExportedBatch/eventIds``.
    public func exportPendingBatch() throws -> ExportedBatch? {
        do {
            guard let ffi = try vault.exportPendingBatch() else { return nil }
            return ExportedBatch(
                batchId: ffi.batchId,
                deviceId: ffi.deviceId,
                ciphertext: Data(ffi.ciphertext),
                eventIds: ffi.eventIds
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }

    /// Apply a downloaded encrypted batch blob against local state.
    ///
    /// Decrypts, three-way merges, applies cleanly-merged events, and persists
    /// any conflicts for review (retrievable via ``listConflicts()``). Returns
    /// the applied / conflict / skipped counts. Idempotent.
    public func ingestBatch(ciphertext: Data) throws -> IngestOutcome {
        do {
            let ffi = try vault.ingestBatch(ciphertext: [UInt8](ciphertext))
            return IngestOutcome(
                applied: ffi.applied,
                conflicts: ffi.conflicts,
                skipped: ffi.skipped
            )
        } catch let error as LdgrError {
            throw LdgrClientError(from: error)
        }
    }
}
