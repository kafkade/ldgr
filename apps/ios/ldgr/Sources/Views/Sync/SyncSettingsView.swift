import SwiftUI
import LdgrSwift

/// Full sync settings and status view.
///
/// Shows sync status, device ID, pending events, conflicts,
/// and a manual sync trigger button.
struct SyncSettingsView: View {
    let client: LdgrClient
    let syncManager: SyncManager

    var body: some View {
        List {
            statusSection
            actionsSection

            if !syncManager.conflicts.isEmpty {
                conflictsSection
            }

            deviceSection
        }
        .navigationTitle("Sync")
        .task {
            await syncManager.refreshStatus(client: client)
        }
        .refreshable {
            await syncManager.refreshStatus(client: client)
        }
    }

    // MARK: - Sections

    private var statusSection: some View {
        Section("Status") {
            if let status = syncManager.status {
                LabeledContent("Pending Changes") {
                    Text("\(status.pendingEventCount)")
                        .foregroundStyle(status.pendingEventCount > 0 ? .blue : .secondary)
                }

                LabeledContent("Unresolved Conflicts") {
                    Text("\(status.unresolvedConflictCount)")
                        .foregroundStyle(status.unresolvedConflictCount > 0 ? .orange : .secondary)
                }

                if let lastSync = status.lastSyncAt {
                    LabeledContent("Last Sync") {
                        Text(String(lastSync.prefix(19)))
                            .font(.caption)
                    }
                } else {
                    LabeledContent("Last Sync") {
                        Text("Never")
                            .foregroundStyle(.secondary)
                    }
                }
            } else {
                HStack {
                    ProgressView()
                    Text("Loading…")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var actionsSection: some View {
        Section {
            Button {
                Task { await syncManager.sync(client: client) }
            } label: {
                HStack {
                    Label("Sync Now", systemImage: "arrow.triangle.2.circlepath")
                    Spacer()
                    if syncManager.isSyncing {
                        ProgressView()
                    }
                }
            }
            .disabled(syncManager.isSyncing)
        }
    }

    private var conflictsSection: some View {
        Section("Conflicts (\(syncManager.conflicts.count))") {
            NavigationLink {
                ConflictListView(
                    client: client,
                    syncManager: syncManager
                )
            } label: {
                Label(
                    "\(syncManager.conflicts.count) conflict\(syncManager.conflicts.count == 1 ? "" : "s") to review",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
            }
        }
    }

    private var deviceSection: some View {
        Section("Device") {
            if let status = syncManager.status {
                LabeledContent("Device ID") {
                    Text(String(status.deviceId.prefix(8)) + "…")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}
