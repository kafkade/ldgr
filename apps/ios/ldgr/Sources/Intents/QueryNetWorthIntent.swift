import AppIntents
import Foundation

/// Siri Shortcut: "What's my net worth?"
/// Reads the cached financial summary from the App Group.
/// No vault decryption — all data is pre-computed.
struct QueryNetWorthIntent: AppIntent {
    static let title: LocalizedStringResource = "Query Net Worth"
    static let description = IntentDescription(
        "Check your current net worth."
    )

    static let openAppWhenRun: Bool = false

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
            return .result(dialog: "No financial data available yet.")
        }

        if summary.netWorth.isEmpty {
            return .result(dialog: "No net worth data available.")
        }

        let lines = summary.netWorth.map { entry in
            "\(entry.amount) \(entry.commodity)"
        }
        let formatted = lines.joined(separator: ", ")
        return .result(
            dialog: "Your net worth is \(formatted)."
        )
    }
}
