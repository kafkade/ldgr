import SwiftUI
import LdgrSwift

/// Router view — shows the appropriate screen based on vault state.
struct ContentView: View {
    @Bindable var appState: AppState
    @Binding var client: LdgrClient?

    var body: some View {
        Group {
            switch appState.status {
            case .noVault:
                VaultSetupView(appState: appState, client: $client)

            case .locked, .unlocking:
                if let client {
                    UnlockView(appState: appState, client: client)
                } else {
                    ProgressView("Initializing…")
                }

            case .unlocked:
                if let client {
                    #if os(macOS)
                    MacRootView(appState: appState, client: client)
                    #else
                    MainTabView(appState: appState, client: client)
                    #endif
                } else {
                    ProgressView("Initializing…")
                }
            }
        }
        .alert("Error", isPresented: .constant(appState.errorMessage != nil)) {
            Button("OK") { appState.errorMessage = nil }
        } message: {
            Text(appState.errorMessage ?? "")
        }
    }
}
