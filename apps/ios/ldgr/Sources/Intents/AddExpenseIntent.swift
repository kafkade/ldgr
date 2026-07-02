import AppIntents
import Foundation

/// Siri Shortcut: "Add expense in ldgr"
///
/// Opens the app to add a transaction. The amount uses String (not Double)
/// to preserve decimal precision for financial data.
///
/// Security: `openAppWhenRun = true` ensures the vault must be unlocked
/// interactively before the transaction can be created. This prevents
/// automation from bypassing vault security.
struct AddExpenseIntent: AppIntent {
    static let title: LocalizedStringResource = "Add Expense"
    static let description = IntentDescription(
        "Open ldgr to record a new expense."
    )

    static let openAppWhenRun: Bool = true

    @Parameter(title: "Amount", description: "Expense amount (e.g. 42.50)")
    var amount: String?

    @Parameter(title: "Description", description: "What was this expense for?")
    var expenseDescription: String?

    @Parameter(title: "Account", description: "Expense account (e.g. Expenses:Food)")
    var account: String?

    func perform() async throws -> some IntentResult {
        // Store intent parameters for the app to pick up on launch
        if let defaults = UserDefaults(
            suiteName: WatchSummary.iosAppGroupId
        ) {
            var pending: [String: String] = [:]
            if let amount { pending["amount"] = amount }
            if let expenseDescription {
                pending["description"] = expenseDescription
            }
            if let account { pending["account"] = account }

            if !pending.isEmpty {
                if let data = try? JSONEncoder().encode(pending) {
                    defaults.set(data, forKey: "pendingExpense")
                }
            }
        }

        return .result()
    }
}
