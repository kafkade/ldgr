import SwiftUI

/// Monthly spending summary with top expense categories.
struct BudgetView: View {
    let budget: WatchSummary.BudgetSummary?

    var body: some View {
        List {
            if let budget {
                Section {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(budget.monthLabel)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Text(budget.totalSpent)
                            .font(.title3.weight(.bold).monospacedDigit())
                        Text("total spending")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }

                if !budget.categories.isEmpty {
                    Section("Top Categories") {
                        ForEach(budget.categories, id: \.name) { cat in
                            HStack {
                                Text(cat.name)
                                    .font(.caption)
                                    .lineLimit(1)
                                Spacer()
                                Text(cat.amount)
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.orange)
                            }
                        }
                    }
                }
            } else {
                Text("No spending data")
                    .foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Spending")
    }
}
