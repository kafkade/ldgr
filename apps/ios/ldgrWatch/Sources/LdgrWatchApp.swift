import SwiftUI

@main
struct LdgrWatchApp: App {
    @State private var connectivity = PhoneConnectivityManager()

    var body: some Scene {
        WindowGroup {
            WatchHomeView(connectivity: connectivity)
                .onAppear {
                    connectivity.requestUpdate()
                }
        }
    }
}
