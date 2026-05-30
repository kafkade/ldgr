import XCTest
@testable import LdgrSwift

/// Smoke test matching the acceptance criteria:
/// init vault → add transaction → query balance.
///
/// These tests require the XCFramework to be built first:
///   cd bindings/swift && ./build-xcframework.sh
final class LdgrSwiftTests: XCTestCase {

    func testVaultLifecycle() async throws {
        let tmpDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tmpDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmpDir) }

        let client = try LdgrClient(path: tmpDir.path)

        // 1. Init vault
        let recoveryKey = try await client.createVault(password: "test123", name: "Test Vault")
        XCTAssertFalse(recoveryKey.isEmpty, "Recovery key should be non-empty")
        XCTAssertTrue(client.isUnlocked)
        XCTAssertEqual(try client.vaultName(), "Test Vault")

        // 2. Create accounts
        let checkingId = try client.addAccount(name: "Assets:Checking", type: .asset, commodity: "USD")
        let foodId = try client.addAccount(name: "Expenses:Food", type: .expense, commodity: "USD")
        XCTAssertFalse(checkingId.isEmpty)

        // 3. Add transaction
        let txId = try client.addTransaction(
            date: "2024-06-15",
            description: "Grocery store",
            status: .cleared,
            postings: [
                NewPosting(accountId: checkingId, amount: "-50.00", commodity: "USD"),
                NewPosting(accountId: foodId, amount: "50.00", commodity: "USD"),
            ]
        )
        XCTAssertFalse(txId.isEmpty)

        // 4. Query balance
        let balances = try client.balance()
        XCTAssertFalse(balances.isEmpty, "Balance should have entries")

        // 5. Lock and unlock
        client.close()
        XCTAssertFalse(client.isUnlocked)

        try await client.open(password: "test123")
        XCTAssertTrue(client.isUnlocked)

        // Data persisted
        let txns = try client.listTransactions()
        XCTAssertEqual(txns.count, 1)
        XCTAssertEqual(txns[0].description, "Grocery store")
    }

    func testWrongPassword() async throws {
        let tmpDir = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: tmpDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmpDir) }

        let client = try LdgrClient(path: tmpDir.path)
        _ = try await client.createVault(password: "correct", name: "Test")
        client.close()

        do {
            try await client.open(password: "wrong")
            XCTFail("Should have thrown")
        } catch let error as LdgrClientError {
            if case .invalidPassword = error {
                // Expected
            } else {
                XCTFail("Expected invalidPassword, got \(error)")
            }
        }
    }
}
