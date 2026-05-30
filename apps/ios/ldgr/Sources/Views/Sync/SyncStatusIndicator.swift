import SwiftUI
import LdgrSwift

/// Compact sync status indicator for the toolbar.
///
/// Shows pending event count and conflict badge. Taps navigate
/// to the full sync settings view.
struct SyncStatusIndicator: View {
    let syncManager: SyncManager

    var body: some View {
        HStack(spacing: 4) {
            if syncManager.isSyncing {
                ProgressView()
                    .controlSize(.mini)
            } else if let status = syncManager.status {
                if status.unresolvedConflictCount > 0 {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.caption)
                } else if status.pendingEventCount > 0 {
                    Image(systemName: "arrow.triangle.2.circlepath")
                        .foregroundStyle(.blue)
                        .font(.caption)
                } else {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .font(.caption)
                }
            }
        }
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        guard let status = syncManager.status else { return "Sync status unknown" }
        if syncManager.isSyncing { return "Syncing" }
        if status.unresolvedConflictCount > 0 {
            return "\(status.unresolvedConflictCount) conflicts need review"
        }
        if status.pendingEventCount > 0 {
            return "\(status.pendingEventCount) changes pending sync"
        }
        return "Synced"
    }
}
