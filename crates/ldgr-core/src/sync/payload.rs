//! Canonical sync event payloads — the single source of truth for the bytes
//! carried inside a [`SyncEvent::payload`](super::events::SyncEvent).
//!
//! These structs are shared by BOTH the outbox emitters (the `_with_sync`
//! storage variants that serialize them) and the apply path (which deserializes
//! them and writes the canonical tables). Keeping one schema here prevents the
//! emit side and the apply side from drifting — a mismatch would silently drop
//! or corrupt financial fields across devices.
//!
//! Per ADR-003, events are **transaction-atomic** and carry the **full entity
//! state** so a remote device can reproduce the entity byte-for-byte. `Create`
//! and `Update` operations serialize the full entity payload; `Delete`
//! operations serialize only [`DeletePayload`] (the entity id).
//!
//! Wire format: each payload is `serde_json` (UTF-8). Field order is fixed by
//! the struct definitions, so the encoding is stable across CLI / FFI / WASM.

use serde::{Deserialize, Serialize};

/// Full state of an account, carried by `Create`/`Update` account events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPayload {
    pub id: String,
    pub name: String,
    /// Canonical account-type string (`asset`/`liability`/`income`/`expense`/`equity`).
    pub account_type: String,
    pub commodity: Option<String>,
    pub parent_id: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub modified_at: String,
}

/// Full state of a posting within a [`TransactionPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostingPayload {
    pub id: String,
    pub account_id: String,
    pub amount_quantity: Option<String>,
    pub amount_commodity: Option<String>,
    pub balance_assertion_quantity: Option<String>,
    pub balance_assertion_commodity: Option<String>,
    pub created_at: String,
    pub version: i64,
}

/// Full state of a transaction (with all its postings), carried by
/// `Create`/`Update` transaction events. The posting list order is significant
/// and is reproduced as the `posting_order` on apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionPayload {
    pub id: String,
    pub date: String,
    /// Canonical status string (`unmarked`/`pending`/`cleared`).
    pub status: String,
    pub code: Option<String>,
    pub description: String,
    pub comment: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub postings: Vec<PostingPayload>,
}

/// Full state of a single allocation within a [`BudgetPayload`].
///
/// Carries only the observable [`crate::budget::BudgetAllocation`] fields. The
/// allocation row's internal id / `created_at` / `version` are regenerated on
/// every write and never surfaced by `get_budget`, so they are re-generated on
/// apply rather than carried across the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationPayload {
    pub account: String,
    /// `Decimal` amount serialized as its canonical string form.
    pub amount: String,
    pub rollover: bool,
}

/// Full state of a budget (with all its allocations), carried by
/// `Create`/`Update` budget events. The allocation list order is significant and
/// is reproduced as the `allocation_order` on apply, exactly as
/// [`TransactionPayload`] reproduces `posting_order`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetPayload {
    pub id: String,
    pub name: String,
    /// Canonical budget-method string (`envelope`/`zero_based`).
    pub method: String,
    /// Canonical budget-period string (`monthly`/`weekly`/`quarterly`/`annual`).
    pub period: String,
    pub created_at: String,
    pub modified_at: String,
    pub allocations: Vec<AllocationPayload>,
}

/// Payload for a `Delete` event — only the entity id is needed for a soft delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletePayload {
    pub id: String,
}

/// Serialize a payload to canonical JSON bytes.
pub fn to_bytes<T: Serialize>(payload: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(payload)
}

/// Deserialize a payload from JSON bytes.
pub fn from_bytes<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_payload_round_trip() {
        let p = AccountPayload {
            id: "acc1".into(),
            name: "Assets:Cash".into(),
            account_type: "asset".into(),
            commodity: Some("USD".into()),
            parent_id: Some("parent1".into()),
            note: Some("petty cash".into()),
            created_at: "2024-01-15T00:00:00Z".into(),
            modified_at: "2024-01-16T00:00:00Z".into(),
        };
        let bytes = to_bytes(&p).unwrap();
        let restored: AccountPayload = from_bytes(&bytes).unwrap();
        assert_eq!(p, restored);
    }

    #[test]
    fn transaction_payload_round_trip() {
        let p = TransactionPayload {
            id: "txn1".into(),
            date: "2024-01-15".into(),
            status: "cleared".into(),
            code: Some("REF-1".into()),
            description: "Lunch".into(),
            comment: Some("with team".into()),
            created_at: "2024-01-15T00:00:00Z".into(),
            modified_at: "2024-01-15T00:00:00Z".into(),
            postings: vec![PostingPayload {
                id: "p1".into(),
                account_id: "acc1".into(),
                amount_quantity: Some("-10.00".into()),
                amount_commodity: Some("USD".into()),
                balance_assertion_quantity: Some("90.00".into()),
                balance_assertion_commodity: Some("USD".into()),
                created_at: "2024-01-15T00:00:00Z".into(),
                version: 1,
            }],
        };
        let bytes = to_bytes(&p).unwrap();
        let restored: TransactionPayload = from_bytes(&bytes).unwrap();
        assert_eq!(p, restored);
    }

    #[test]
    fn budget_payload_round_trip() {
        let p = BudgetPayload {
            id: "bud1".into(),
            name: "Monthly".into(),
            method: "envelope".into(),
            period: "monthly".into(),
            created_at: "2024-01-15T00:00:00Z".into(),
            modified_at: "2024-01-16T00:00:00Z".into(),
            allocations: vec![
                AllocationPayload {
                    account: "Expenses:Food".into(),
                    amount: "500.00".into(),
                    rollover: true,
                },
                AllocationPayload {
                    account: "Expenses:Rent".into(),
                    amount: "1200".into(),
                    rollover: false,
                },
            ],
        };
        let bytes = to_bytes(&p).unwrap();
        let restored: BudgetPayload = from_bytes(&bytes).unwrap();
        assert_eq!(p, restored);
    }

    #[test]
    fn delete_payload_round_trip() {
        let p = DeletePayload { id: "x".into() };
        let bytes = to_bytes(&p).unwrap();
        let restored: DeletePayload = from_bytes(&bytes).unwrap();
        assert_eq!(p, restored);
    }
}
