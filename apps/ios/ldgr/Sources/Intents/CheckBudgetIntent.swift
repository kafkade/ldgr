import AppIntents
import Foundation

/// Siri Shortcut: "How much have I spent this month?"
/// Reads the cached spending summary from the App Group.
struct CheckBudgetIntent: AppIntent {
    static var title: LocalizedStringResource = "Check Monthly Spending"
    static var description = IntentDescription(
        "See how much you've spent this month."
    )

    static var openAppWhenRun: Bool = false

    func perform() async throws -> some IntentResult & ProvidesDialog {
        guard let defaults = UserDefaults(
            suiteName: WatchSummary.iosAppGroupId
        ) else {
            return .result(dialog: "Unable to access financial data.")
        }

        let unlocked = defaults.bool(
            forKey: WatchSummary.iosUnlockedKey
        )
        guard unlocked else {
            return .result(dialog: "Please unlock ldgr first.")
        }

        guard let data = defaults.data(
            forKey: WatchSummary.iosDefaultsKey
        ),
            let summary = try? JSONDecoder().decode(
                WatchSummary.self, from: data
            )
        else {
            return .result(dialog: "No spending data available yet.")
        }

        guard let budget = summary.budget else {
            return .result(dialog: "No spending data for this month.")
        }

        var response = "You've spent \(budget.totalSpent) in \(budget.monthLabel)."

        if !budget.categories.isEmpty {
            let top = budget.categories.prefix(3).map { cat in
                "\(cat.name): \(cat.amount)"
            }.joined(separator: ", ")
            response += " Top categories: \(top)."
        }

        if let daily = summary.dailySpend {
            response += " Today: \(daily)."
        }

        return .result(dialog: "\(response)")
    }
}
