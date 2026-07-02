import CoreImage.CIFilterBuiltins
import SwiftUI
import LdgrSwift

#if canImport(UIKit)
import UIKit
#endif

/// Shown **once** after a successful two-secret sign-up (ADR-008).
///
/// The Emergency Kit is the only artefact that lets the user sign in on a new
/// device: it pairs the account address/email with the account **Secret Key**
/// and a scannable QR payload. The Secret Key is never shown again — the user
/// must save the kit (share sheet / screenshot / print) before continuing.
struct EmergencyKitView: View {
    let kit: EmergencyKit
    let onContinue: () -> Void

    @State private var hasConfirmed = false
    @State private var hasCopied = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    header
                    qrSection
                    detailsSection
                    warningSection
                    confirmSection
                }
                .padding()
            }
            .navigationTitle("Emergency Kit")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    ShareLink(item: shareText) {
                        Label("Save", systemImage: "square.and.arrow.up")
                    }
                }
            }
        }
    }

    // MARK: - Sections

    private var header: some View {
        VStack(spacing: 12) {
            Image(systemName: "lifepreserver.fill")
                .font(.system(size: 48))
                .foregroundStyle(.blue)

            Text("Save Your Emergency Kit")
                .font(.title2.weight(.bold))
                .multilineTextAlignment(.center)

            Text("You'll need your **Secret Key** to sign in on any other device — together with your password. Save this kit now; it won't be shown again.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.top, 8)
    }

    @ViewBuilder
    private var qrSection: some View {
        if let image = Self.qrImage(from: kit.qrPayload) {
            VStack(spacing: 8) {
                image
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 220, maxHeight: 220)
                    .padding()
                    .background(Color.white)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                Text("Scan on your new device to sign in.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var detailsSection: some View {
        VStack(spacing: 12) {
            kitRow(label: "Server", value: kit.address)
            kitRow(label: "Account", value: kit.email)
            kitRow(label: "Account Hint", value: kit.accountHint, mono: true)
            secretRow(label: "Secret Key", value: kit.secretKey)
            if let recovery = kit.recoveryKey {
                secretRow(label: "Recovery Key", value: recovery)
            }
        }
    }

    private func kitRow(label: String, value: String, mono: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value)
                .font(mono ? .system(.body, design: .monospaced) : .body)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func secretRow(label: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .padding()
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.ultraThinMaterial)
                .clipShape(RoundedRectangle(cornerRadius: 10))
        }
    }

    private var warningSection: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 4) {
                Text("Your Secret Key won't be shown again.")
                    .font(.subheadline.weight(.semibold))
                Text("Without it you can't add new devices. Store it somewhere safe — a password manager or printed copy.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding()
        .background(.orange.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    private var confirmSection: some View {
        VStack(spacing: 16) {
            Button {
                copySecretKey()
            } label: {
                Label(hasCopied ? "Secret Key Copied" : "Copy Secret Key",
                      systemImage: hasCopied ? "checkmark" : "doc.on.doc")
            }
            .buttonStyle(.bordered)

            Toggle(isOn: $hasConfirmed) {
                Text("I've saved my Emergency Kit")
                    .font(.subheadline)
            }

            Button("Continue", action: onContinue)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(!hasConfirmed)
        }
    }

    // MARK: - Helpers

    private var shareText: String {
        var lines = [
            "ldgr Emergency Kit",
            "Server: \(kit.address)",
            "Account: \(kit.email)",
            "Account Hint: \(kit.accountHint)",
            "Secret Key: \(kit.secretKey)",
        ]
        if let recovery = kit.recoveryKey {
            lines.append("Recovery Key: \(recovery)")
        }
        lines.append("")
        lines.append("Keep this safe. You need your Secret Key AND your password to sign in on a new device.")
        return lines.joined(separator: "\n")
    }

    private func copySecretKey() {
        #if canImport(UIKit)
        UIPasteboard.general.string = kit.secretKey
        #endif
        hasCopied = true
    }

    /// Render `string` into a crisp QR `Image` using CoreImage.
    static func qrImage(from string: String) -> Image? {
        let context = CIContext()
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        #if canImport(UIKit)
        return Image(uiImage: UIImage(cgImage: cgImage))
        #elseif canImport(AppKit)
        return Image(nsImage: NSImage(cgImage: cgImage, size: .zero))
        #else
        return nil
        #endif
    }
}
