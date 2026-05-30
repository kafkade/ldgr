import SwiftUI

/// Displays the recovery key after vault creation.
///
/// This is the ONLY time the recovery key is shown — the core cannot
/// retrieve it later. The user must save it before dismissing.
struct RecoveryKeyView: View {
    let recoveryKey: String
    let onDismiss: () -> Void

    @State private var hasCopied = false
    @State private var hasConfirmed = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    // Header
                    VStack(spacing: 12) {
                        Image(systemName: "key.fill")
                            .font(.system(size: 48))
                            .foregroundStyle(.orange)

                        Text("Recovery Key")
                            .font(.title.weight(.bold))

                        Text("Save this key in a safe place. If you forget your master password, this is the **only way** to recover your vault.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                    .padding(.top, 16)

                    // Recovery key display
                    VStack(spacing: 8) {
                        Text(recoveryKey)
                            .font(.system(.body, design: .monospaced))
                            .padding()
                            .frame(maxWidth: .infinity)
                            .background(.ultraThinMaterial)
                            .clipShape(RoundedRectangle(cornerRadius: 12))

                        HStack(spacing: 16) {
                            Button {
                                UIPasteboard.general.string = recoveryKey
                                hasCopied = true
                            } label: {
                                Label(
                                    hasCopied ? "Copied" : "Copy",
                                    systemImage: hasCopied ? "checkmark" : "doc.on.doc"
                                )
                            }
                            .buttonStyle(.bordered)

                            ShareLink(item: recoveryKey) {
                                Label("Share", systemImage: "square.and.arrow.up")
                            }
                            .buttonStyle(.bordered)
                        }
                    }

                    // Warning
                    HStack(alignment: .top, spacing: 12) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        VStack(alignment: .leading, spacing: 4) {
                            Text("This key will not be shown again.")
                                .font(.subheadline.weight(.semibold))
                            Text("Lost password + lost recovery key = unrecoverable data. This is by design.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding()
                    .background(.orange.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 12))

                    // Confirmation
                    Toggle(isOn: $hasConfirmed) {
                        Text("I have saved my recovery key")
                            .font(.subheadline)
                    }
                    .padding(.horizontal)

                    Button("Continue") {
                        onDismiss()
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!hasConfirmed)
                }
                .padding()
            }
            .navigationTitle("Save Recovery Key")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}
