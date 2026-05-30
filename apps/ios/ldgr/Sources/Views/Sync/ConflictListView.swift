import SwiftUI
import LdgrSwift

/// List of unresolved sync conflicts requiring user review.
///
/// Each row shows the entity type, ID, and when the conflict was detected.
/// Tapping a row navigates to `ConflictDetailView` for side-by-side comparison.
struct ConflictListView: View {
    let client: LdgrClient
    let syncManager: SyncManager

    var body: some View {
        Group {
            if syncManager.conflicts.isEmpty {
                ContentUnavailableView(
                    "No Conflicts",
                    systemImage: "checkmark.circle",
                    description: Text("All sync conflicts have been resolved.")
                )
            } else {
                List(syncManager.conflicts) { conflict in
                    NavigationLink {
                        ConflictDetailView(
                            client: client,
                            syncManager: syncManager,
                            conflict: conflict
                        )
                    } label: {
                        ConflictRow(conflict: conflict)
                    }
                }
            }
        }
        .navigationTitle("Conflicts")
    }
}

/// A single conflict row showing entity type and detection date.
private struct ConflictRow: View {
    let conflict: SyncConflict

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: iconName)
                    .foregroundStyle(.orange)
                Text(conflict.entityType.capitalized)
                    .font(.headline)
            }

            Text("Entity: \(conflict.entityId.prefix(8))…")
                .font(.caption)
                .foregroundStyle(.secondary)

            Text("Detected: \(formattedDate)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }

    private var iconName: String {
        switch conflict.entityType {
        case "transaction": "arrow.left.arrow.right"
        case "account": "building.columns"
        default: "questionmark.circle"
        }
    }

    private var formattedDate: String {
        // ISO 8601 dates — show just the date portion for readability
        String(conflict.detectedAt.prefix(10))
    }
}
