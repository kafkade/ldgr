#if os(macOS)
import SwiftUI

// macOS menu bar + keyboard-shortcut plumbing.
//
// The menu bar lives at the `App` scene level, but the actions it triggers
// (add a transaction, sync, lock) belong to whichever vault window is key.
// We bridge the two with a `FocusedValue`: the key window publishes its
// `LdgrWindowActions` via `.focusedSceneValue`, and `LdgrCommands` reads it.
// When no unlocked vault window is focused the value is `nil`, so the menu
// items disable themselves automatically.

/// Actions the frontmost unlocked vault window exposes to the macOS menu bar.
struct LdgrWindowActions {
    /// Present the new-transaction form.
    var newTransaction: () -> Void
    /// Present the journal import picker.
    var importJournal: () -> Void
    /// Lock the vault (closes the client, returns to the unlock screen).
    var lock: () -> Void
    /// Kick off a sync cycle.
    var sync: () -> Void
    /// Whether "Sync Now" should be enabled (disabled while a sync is running).
    var canSync: Bool
}

private struct LdgrWindowActionsKey: FocusedValueKey {
    typealias Value = LdgrWindowActions
}

extension FocusedValues {
    /// Actions published by the frontmost unlocked vault window, if any.
    var ldgrWindowActions: LdgrWindowActions? {
        get { self[LdgrWindowActionsKey.self] }
        set { self[LdgrWindowActionsKey.self] = newValue }
    }
}

/// macOS menu commands: a customized **File** menu (New Transaction, Import,
/// New Window) plus a dedicated **Vault** menu (Sync Now, Lock Vault).
struct LdgrCommands: Commands {
    @FocusedValue(\.ldgrWindowActions) private var actions
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        // Replace the stock "New" group so ⌘N creates a transaction (the most
        // common action) while multi-window moves to ⇧⌘N.
        CommandGroup(replacing: .newItem) {
            Button("New Transaction") { actions?.newTransaction() }
                .keyboardShortcut("n", modifiers: .command)
                .disabled(actions == nil)

            Button("Import…") { actions?.importJournal() }
                .keyboardShortcut("i", modifiers: .command)
                .disabled(actions == nil)

            Divider()

            Button("New Window") { openWindow(id: LdgrScene.main) }
                .keyboardShortcut("n", modifiers: [.command, .shift])
        }

        CommandMenu("Vault") {            Button("Sync Now") { actions?.sync() }
                .keyboardShortcut("r", modifiers: .command)
                .disabled(actions?.canSync != true)

            Divider()

            Button("Lock Vault") { actions?.lock() }
                .keyboardShortcut("l", modifiers: .command)
                .disabled(actions == nil)
        }
    }
}
#endif
