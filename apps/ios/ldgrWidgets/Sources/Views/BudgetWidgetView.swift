import SwiftUI
import WidgetKit

struct BudgetWidgetView: View {
    let entry: SpendingEntry

    var body: some View {
        if entry.isLocked {
            LockedPlaceholderView()
        } else if let budget = entry.budget {
            spendingLayout(budget)
        } else {
            NoDataPlaceholderView(label: "Spending")
        }
    }

    private func spendingLayout(
        _ budget: WatchSummary.BudgetSummary
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Label("Monthly Spending", systemImage: "creditcard")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Text(budget.monthLabel)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }

            Text(budget.totalSpent)
                .font(.title3)
                .fontWeight(.bold)
                .monospacedDigit()

            if let daily = entry.dailySpend {
                Text("Today: \(daily)")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            Divider()

            ForEach(
                budget.categories.prefix(3),
                id: \.name
            ) { category in
                HStack {
                    Text(category.name)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer()
                    Text(category.amount)
                        .font(.caption2)
                        .monospacedDigit()
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(4)
    }
}
