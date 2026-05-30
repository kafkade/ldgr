import SwiftUI
import LdgrSwift

/// Side-by-side comparison of conflicting local and remote versions.
///
/// Shows the JSON payloads in a readable format and provides
/// Keep Local / Keep Remote resolution buttons.
struct ConflictDetailView: View {
    let client: LdgrClient
    let syncManager: SyncManager
    let conflict: SyncConflict

    @Environment(\.dismiss) private var dismiss
    @State private var isResolving = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header

                HStack(alignment: .top, spacing: 16) {
                    payloadCard(title: "Local", payload: conflict.localPayload, color: .blue)
                    payloadCard(title: "Remote", payload: conflict.remotePayload, color: .purple)
                }

                resolutionButtons
            }
            .padding()
        }
        .navigationTitle("Conflict Detail")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Subviews

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(conflict.entityType.capitalized, systemImage: "exclamationmark.triangle.fill")
                .font(.title3.bold())
                .foregroundStyle(.orange)

            Text("Entity ID: \(conflict.entityId)")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)

            Text("Detected: \(String(conflict.detectedAt.prefix(19)))")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
    }

    private func payloadCard(title: String, payload: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.subheadline.bold())
                .foregroundStyle(color)

            Text(formatPayload(payload))
                .font(.caption.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
                .background(Color(.systemGray6))
                .clipShape(RoundedRectangle(cornerRadius: 8))
        }
        .frame(maxWidth: .infinity)
    }

    private var resolutionButtons: some View {
        HStack(spacing: 16) {
            Button {
                Task { await resolve(.keepLocal) }
            } label: {
                Label("Keep Local", systemImage: "iphone")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .tint(.blue)

            Button {
                Task { await resolve(.keepRemote) }
            } label: {
                Label("Keep Remote", systemImage: "cloud")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .tint(.purple)
        }
        .disabled(isResolving)
    }

    // MARK: - Actions

    private func resolve(_ resolution: ConflictResolution) async {
        isResolving = true
        await syncManager.resolveConflict(
            client: client,
            conflictId: conflict.id,
            resolution: resolution
        )
        isResolving = false
        dismiss()
    }

    // MARK: - Helpers

    private func formatPayload(_ json: String) -> String {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data),
              let pretty = try? JSONSerialization.data(withJSONObject: obj, options: .prettyPrinted),
              let str = String(data: pretty, encoding: .utf8) else {
            return json
        }
        return str
    }
}
