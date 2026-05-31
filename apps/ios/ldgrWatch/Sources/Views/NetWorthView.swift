import SwiftUI

/// Detail view showing net worth breakdown by commodity.
struct NetWorthView: View {
    let entries: [WatchSummary.NetWorthEntry]

    var body: some View {
        List {
            if entries.isEmpty {
                Text("No balance data")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(entries, id: \.commodity) { entry in
                    HStack {
                        Text(entry.commodity)
                            .font(.caption.weight(.medium))
                        Spacer()
                        Text(entry.amount)
                            .font(.body.monospacedDigit())
                    }
                }
            }
        }
        .navigationTitle("Net Worth")
    }
}
