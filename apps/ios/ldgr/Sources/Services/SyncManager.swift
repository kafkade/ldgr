import Foundation
import LdgrSwift

/// Manages sync state and operations, observed by the sync UI.
///
/// Provides a reactive surface for sync status, pending events, and
/// unresolved conflicts. The actual transport (server push/pull) is
/// injected via `SyncTransport` protocol — currently a no-op mock
/// since the server doesn't exist yet.
@Observable
final class SyncManager {
    private(set) var status: SyncStatus?
    private(set) var conflicts: [SyncConflict] = []
    private(set) var isSyncing = false
    var errorMessage: String?

    /// Protocol for sync transport — injected for testability.
    protocol SyncTransport {
        func push(events: [SyncEvent]) async throws -> [String]
        func pull() async throws -> [SyncConflict]
    }

    /// Mock transport for development — always succeeds with no data.
    struct MockSyncTransport: SyncTransport {
        func push(events: [SyncEvent]) async throws -> [String] {
            events.map(\.id)
        }

        func pull() async throws -> [SyncConflict] {
            []
        }
    }

    private let transport: SyncTransport

    init(transport: SyncTransport = MockSyncTransport()) {
        self.transport = transport
    }

    /// Refresh sync status from the vault.
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

    /// Perform a full sync cycle: push pending events, pull remote changes.
    func sync(client: LdgrClient) async {
        guard !isSyncing else { return }
        isSyncing = true
        defer { isSyncing = false }

        do {
            // Push pending events
            let events = try await Task.detached {
                try client.pendingSyncEvents()
            }.value

            if !events.isEmpty {
                let syncedIds = try await transport.push(events: events)
                if !syncedIds.isEmpty {
                    try await Task.detached {
                        try client.markEventsSynced(eventIds: syncedIds)
                    }.value
                }
            }

            // Pull remote changes (may produce conflicts)
            let newConflicts = try await transport.pull()
            if !newConflicts.isEmpty {
                // Store any new conflicts for user review
                let ffiConflicts = newConflicts.map { c in
                    LdgrSwift.SyncConflict(
                        id: c.id,
                        entityType: c.entityType,
                        entityId: c.entityId,
                        localPayload: c.localPayload,
                        remotePayload: c.remotePayload,
                        detectedAt: c.detectedAt
                    )
                }
                _ = ffiConflicts // Conflicts already structured
            }

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
}
