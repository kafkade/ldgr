import SwiftUI
import LdgrSwift

/// Stable scene identifiers used for multi-window support.
enum LdgrScene {
    static let main = "main"
}

/// App entry point. Monitors scene phase for auto-lock and privacy protection.
@main
struct LdgrApp: App {
    @State private var appState = AppState()
    @State private var client: LdgrClient?
    @State private var watchManager = WatchConnectivityManager()
    @State private var widgetManager = WidgetDataManager()
    @Environment(\.scenePhase) private var scenePhase

    /// Overlay to hide sensitive content in the app switcher.
    @State private var showPrivacyOverlay = false

    var body: some Scene {
        WindowGroup(id: LdgrScene.main) {
            ZStack {
                ContentView(appState: appState, client: $client)
                    .environment(watchManager)
                    .environment(widgetManager)

                if showPrivacyOverlay {
                    PrivacyOverlayView()
                        .transition(.opacity)
                }
            }
            .animation(.easeInOut(duration: 0.15), value: showPrivacyOverlay)
            .onAppear {
                initializeClient()
            }
            .onChange(of: scenePhase) { oldPhase, newPhase in
                handleScenePhase(from: oldPhase, to: newPhase)
            }
        }
        #if os(macOS)
        .defaultSize(width: 1040, height: 680)
        .windowResizability(.contentMinSize)
        .commands {
            LdgrCommands()
        }
        #endif
    }

    private func initializeClient() {
        do {
            client = try LdgrClient(path: appState.vaultPath)
        } catch {
            appState.setError("Failed to initialize vault: \(error.localizedDescription)")
        }
    }

    private func handleScenePhase(from oldPhase: ScenePhase, to newPhase: ScenePhase) {
        switch newPhase {
        case .inactive:
            // App switcher / control center — hide content
            showPrivacyOverlay = true

        case .background:
            // Definitely backgrounded — record timestamp and lock
            showPrivacyOverlay = true
            appState.didEnterBackground()
            if appState.autoLockInterval == .immediate {
                lockVault()
            }

        case .active:
            showPrivacyOverlay = false
            if appState.shouldLockOnForeground() {
                lockVault()
            }

        @unknown default:
            break
        }
    }

    private func lockVault() {
        guard appState.status == .unlocked else { return }
        client?.close()
        widgetManager.clearOnLock()
        appState.transitionToLocked()
    }
}

// MARK: - Privacy Overlay

/// Shown in the app switcher to prevent sensitive data from being visible.
struct PrivacyOverlayView: View {
    var body: some View {
        ZStack {
            Color.platformBackground
                .ignoresSafeArea()
            VStack(spacing: 12) {
                Image(systemName: "lock.shield.fill")
                    .font(.system(size: 48))
                    .foregroundStyle(.secondary)
                Text("ldgr")
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
        }
    }
}
