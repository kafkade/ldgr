import SwiftUI

/// Shown when no summary has been received from the iPhone companion yet.
struct NoDataView: View {
    let connectivity: PhoneConnectivityManager

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "iphone.and.arrow.right.inward")
                .font(.system(size: 32))
                .foregroundStyle(.secondary)

            Text("Waiting for Data")
                .font(.headline)

            Text("Open ldgr on your iPhone to sync financial summaries.")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            if connectivity.isReachable {
                Button("Refresh") {
                    connectivity.requestUpdate()
                }
                .buttonStyle(.bordered)
                .tint(.blue)
            }
        }
        .padding()
    }
}
