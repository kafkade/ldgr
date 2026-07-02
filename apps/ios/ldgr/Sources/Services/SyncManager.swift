import Foundation
import LdgrSwift

/// Manages sync state and operations, observed by the sync UI.
///
/// Sync is a two-phase cycle over **encrypted batch blobs** (issues #200/#201):
/// the vault composes pending events into a blob (`exportPendingBatch`), the
/// transport uploads/downloads opaque blobs, and the vault applies downloaded
/// blobs (`ingestBatch`, which persists conflicts for review). The network seam
/// is the injected ``SyncTransport`` — a real ``ServerSyncTransport`` when the
/// user has configured + authenticated a server, otherwise ``MockSyncTransport``
/// (the default, used for previews and tests).
@MainActor
@Observable
final class SyncManager {
    private(set) var status: SyncStatus?
    private(set) var conflicts: [SyncConflict] = []
    private(set) var isSyncing = false
    private(set) var isServerConfigured = false
    var errorMessage: String?

    /// Network seam for sync — uploads/downloads opaque encrypted batch blobs.
    ///
    /// Deliberately blob-oriented (not event-oriented): the vault's
    /// `exportPendingBatch`/`ingestBatch` own event composition and conflict
    /// persistence, so the transport only moves bytes. This keeps it trivial to
    /// mock and isolates all networking to ``ServerSyncTransport``.
    protocol SyncTransport: Sendable {
        /// Upload one composed batch blob.
        func push(batchId: String, ciphertext: Data) async throws
        /// List remote batches newer than `since` (excluding our own device),
        /// download each, and return the blobs plus the advanced cursor.
        func fetchRemoteBatches(since: String?) async throws -> (blobs: [Data], cursor: String?)
    }

    /// No-op transport for previews/tests — never produces or consumes data.
    struct MockSyncTransport: SyncTransport {
        func push(batchId: String, ciphertext: Data) async throws {}

        func fetchRemoteBatches(since: String?) async throws -> (blobs: [Data], cursor: String?) {
            ([], since)
        }
    }

    private var transport: SyncTransport
    private var vaultId: String?

    init(transport: SyncTransport = MockSyncTransport(), vaultId: String? = nil) {
        self.transport = transport
        self.vaultId = vaultId
        self.isServerConfigured = !(transport is MockSyncTransport)
    }

    // MARK: - Configuration

    /// Rebuild the transport from persisted ``ServerConfig`` + the Keychain
    /// session token. Falls back to ``MockSyncTransport`` when the server is not
    /// fully configured or the user is not signed in.
    ///
    /// Call this on appear and after a successful sign-in / sign-out.
    func configure(client: LdgrClient) {
        let config = ServerConfigStore.load()
        guard config.isComplete,
              let baseURL = config.baseURLValue,
              let token = KeychainManager.retrieveServerAuthToken(),
              let deviceId = try? client.syncStatus().deviceId
        else {
            transport = MockSyncTransport()
            vaultId = nil
            isServerConfigured = false
            return
        }

        let syncClient = LdgrSync.makeSession(baseURL: baseURL, token: token)
        transport = ServerSyncTransport(
            client: syncClient,
            vaultId: config.vaultId,
            deviceId: deviceId
        )
        vaultId = config.vaultId
        isServerConfigured = true
    }

    // MARK: - Status

    /// Refresh sync status + conflicts from the vault.
    func refreshStatus(client: LdgrClient) async {
        do {
            let result = try await Task.detached {
                let status = try client.syncStatus()
                let conflicts = try client.listConflicts()
                return (status, conflicts)
            }.value

            status = result.0
            conflicts = result.1
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Sync

    /// Perform a full sync cycle: push the pending batch, then pull + apply
    /// remote batches.
    ///
    /// Push: compose pending events into one encrypted blob; if non-empty,
    /// upload it and mark those events synced. Pull: list/fetch remote blobs
    /// (excluding our own device) and ingest each — ingest persists any
    /// conflicts into the vault, which ``refreshStatus(client:)`` then surfaces.
    func sync(client: LdgrClient) async {
        guard !isSyncing else { return }
        isSyncing = true
        defer { isSyncing = false }

        do {
            // ── Push ──────────────────────────────────────────────────────────
            if let batch = try await Task.detached(operation: {
                try client.exportPendingBatch()
            }).value {
                try await transport.push(batchId: batch.batchId, ciphertext: batch.ciphertext)
                let eventIds = batch.eventIds
                try await Task.detached(operation: {
                    try client.markEventsSynced(eventIds: eventIds)
                }).value
            }

            // ── Pull ──────────────────────────────────────────────────────────
            let (blobs, cursor) = try await transport.fetchRemoteBatches(since: currentCursor)
            for blob in blobs {
                _ = try await Task.detached(operation: {
                    try client.ingestBatch(ciphertext: blob)
                }).value
            }
            currentCursor = cursor

            await refreshStatus(client: client)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Resolve a conflict and refresh status.
    func resolveConflict(
        client: LdgrClient,
        conflictId: String,
        resolution: ConflictResolution
    ) async {
        do {
            try await Task.detached {
                try client.resolveConflict(id: conflictId, resolution: resolution)
            }.value
            await refreshStatus(client: client)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Pull cursor (per-vault, persisted)

    private var currentCursor: String? {
        get { vaultId.flatMap { ServerConfigStore.sinceCursor(vaultId: $0) } }
        set {
            guard let vaultId else { return }
            ServerConfigStore.setSinceCursor(newValue, vaultId: vaultId)
        }
    }
}

/// Real network transport backed by the authenticated ``LdgrSyncClient``.
///
/// Holds the vault id and our *local* device id. The device id is used both as
/// the path segment when uploading our batches and to exclude our own batches
/// when pulling (a device never needs to re-ingest what it produced).
struct ServerSyncTransport: SyncManager.SyncTransport {
    let client: LdgrSyncSession
    let vaultId: String
    let deviceId: String

    func push(batchId: String, ciphertext: Data) async throws {
        _ = try await client.putBatch(
            vaultId: vaultId,
            deviceId: deviceId,
            batchId: batchId,
            ciphertext: ciphertext
        )
    }

    func fetchRemoteBatches(since: String?) async throws -> (blobs: [Data], cursor: String?) {
        let metas = try await client.listRemoteBatches(
            vaultId: vaultId,
            since: since,
            deviceId: nil,
            limit: nil
        )

        var blobs: [Data] = []
        var cursor = since
        for meta in metas {
            // Advance the cursor past every batch we observe (including our own)
            // so we don't re-list them next time.
            if let modifiedAt = meta.modifiedAt,
               cursor == nil || modifiedAt > cursor! {
                cursor = modifiedAt
            }
            // Skip batches this device produced — already applied locally.
            guard meta.deviceId != deviceId else { continue }

            let blob = try await client.getBatch(
                vaultId: vaultId,
                deviceId: meta.deviceId,
                batchId: meta.batchId
            )
            blobs.append(blob)
        }
        return (blobs, cursor)
    }
}
