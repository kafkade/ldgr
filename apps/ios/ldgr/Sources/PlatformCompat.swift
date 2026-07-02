import SwiftUI

// Cross-platform helpers so the shared SwiftUI views in `ldgr/Sources`
// compile for both iOS and macOS. iOS-only symbols (UIKit colors, tab-bar
// toolbar placements, `UIPasteboard`) have no direct macOS equivalent, so
// these thin shims pick the right platform API behind `#if`.

extension Color {
    /// Primary window/background fill (`UIColor.systemBackground` on iOS,
    /// `NSColor.windowBackgroundColor` on macOS).
    static var platformBackground: Color {
        #if os(macOS)
        Color(nsColor: .windowBackgroundColor)
        #else
        Color(.systemBackground)
        #endif
    }

    /// Subtle grouped/secondary surface fill.
    static var platformGroupedSecondary: Color {
        #if os(macOS)
        Color(nsColor: .underPageBackgroundColor)
        #else
        Color(.systemGray6)
        #endif
    }
}

extension ToolbarItemPlacement {
    /// Leading edge of the navigation/toolbar area.
    static var platformLeading: ToolbarItemPlacement {
        #if os(macOS)
        .navigation
        #else
        .topBarLeading
        #endif
    }

    /// Trailing edge / primary action area of the navigation/toolbar.
    static var platformTrailing: ToolbarItemPlacement {
        #if os(macOS)
        .primaryAction
        #else
        .topBarTrailing
        #endif
    }

    /// Bottom bar on iOS; falls back to the automatic placement on macOS
    /// (which has no bottom bar).
    static var platformBottomBar: ToolbarItemPlacement {
        #if os(macOS)
        .automatic
        #else
        .bottomBar
        #endif
    }
}

/// Copies text to the system clipboard on the current platform.
enum PlatformClipboard {
    static func copy(_ string: String) {
        #if canImport(UIKit)
        UIPasteboard.general.string = string
        #elseif canImport(AppKit)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(string, forType: .string)
        #endif
    }
}

#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Wraps a non-`Sendable` value so it can be carried across a concurrency
/// boundary when the programmer knows the access is safe (e.g. a callback
/// that is only invoked once). Use sparingly.
struct UncheckedSendable<Value>: @unchecked Sendable {
    let value: Value

    init(_ value: Value) {
        self.value = value
    }
}
