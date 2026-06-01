import AppIntents

/// Registers shortcut phrases for Siri integration.
struct LdgrShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: QueryNetWorthIntent(),
            phrases: [
                "What's my net worth in \(.applicationName)",
                "Check net worth in \(.applicationName)",
                "\(.applicationName) net worth"
            ],
            shortTitle: "Net Worth",
            systemImageName: "banknote"
        )

        AppShortcut(
            intent: CheckBudgetIntent(),
            phrases: [
                "How much have I spent in \(.applicationName)",
                "Check spending in \(.applicationName)",
                "\(.applicationName) monthly spending"
            ],
            shortTitle: "Monthly Spending",
            systemImageName: "creditcard"
        )

        AppShortcut(
            intent: AddExpenseIntent(),
            phrases: [
                "Add expense in \(.applicationName)",
                "Record expense in \(.applicationName)",
                "Log spending in \(.applicationName)"
            ],
            shortTitle: "Add Expense",
            systemImageName: "plus.circle"
        )
    }
}
