# ldgr iOS App

SwiftUI app for zero-knowledge personal finance on iOS and iPadOS.

## Prerequisites

- Xcode 16+ (with the iOS, watchOS and macOS SDKs)
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) for project generation
- Built XCFramework from `bindings/swift/build-xcframework.sh`

The generated `ldgr_ffiFFI.xcframework` bundles slices for iOS (device),
iOS simulator (arm64 + x86_64), macOS (arm64 + x86_64), watchOS (device),
and the watchOS simulator (arm64), so the iOS app and its watchOS target both
link against it.

## Quick Start

```sh
# 1. Build the Rust XCFramework (from repo root)
cd bindings/swift && ./build-xcframework.sh

# 2. Generate the Xcode project
cd apps/ios
brew install xcodegen  # if not installed
xcodegen generate

# 3. Open in Xcode
open ldgr.xcodeproj
```

## Architecture

The app is a thin SwiftUI layer over `LdgrClient` (from `bindings/swift/`):

```text     ┌────────────┐     ┌──────────┐
│  SwiftUI    │ ──► │ LdgrClient │ ──► │ ldgr-ffi │ (Rust, via UniFFI)
│  Views      │     │ (async)    │     │          │
└─────────────┘     └────────────┘     └──────────┘
                          │
                    ┌─────┴──────┐
                    │  Keychain  │ (session key, biometric ACL)
                    └────────────┘
```

### Key Files

| File | Purpose |
| ---- | ------- |
| `Sources/LdgrApp.swift` | App entry, scene phase monitoring, auto-lock |
| `Sources/AppState.swift` | Observable state machine (noVault → locked → unlocked) |
| `Sources/Services/KeychainManager.swift` | Session key storage with biometric access control |
| `Sources/Services/BiometricManager.swift` | Face ID / Touch ID capability detection |
| `Sources/Views/VaultSetupView.swift` | Create vault flow with recovery key display |
| `Sources/Views/UnlockView.swift` | Password + biometric unlock |
| `Sources/Views/HomeView.swift` | Legacy single-screen view (superseded by tab views) |
| `Sources/Views/MainTabView.swift` | Adaptive tab bar (iPhone) / sidebar (iPad) navigation |
| `Sources/Views/Dashboard/DashboardView.swift` | Net worth, recent transactions, quick stats |
| `Sources/Views/Transactions/TransactionListView.swift` | Searchable/filterable transaction list |
| `Sources/Views/Transactions/TransactionFormView.swift` | Add/correct transaction form |
| `Sources/Views/Accounts/AccountListView.swift` | Accounts grouped by type with balances |
| `Sources/Views/Accounts/AccountRegisterView.swift` | Transaction register for a single account |
| `Sources/Views/Investments/InvestmentsView.swift` | Portfolio holdings and allocation chart |
| `Sources/Views/Budget/BudgetView.swift` | Expense category progress bars (current month) |
| `Sources/Services/VaultDataStore.swift` | Shared observable store for vault data across tabs |
| `Sources/Views/SettingsView.swift` | Biometric toggle, auto-lock interval |

### Security Model

- **Session key in Keychain**: After password unlock, the 32-byte vault key is stored
  in the iOS Keychain with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` and
  `SecAccessControl(.biometryCurrentSet)`.
- **Biometric gating**: The Keychain access control itself triggers the biometric
  prompt — there is no separate `LAContext.evaluatePolicy` call.
- **`.biometryCurrentSet`**: If biometrics change (new fingerprint/face), the Keychain
  item is invalidated. The user must re-enter their password.
- **Auto-lock**: The vault is locked when the app enters the background (configurable
  delay: immediate / 1 min / 5 min / 15 min).
- **Privacy overlay**: Sensitive content is hidden in the app switcher via an opaque
  overlay on `scenePhase == .inactive`.
