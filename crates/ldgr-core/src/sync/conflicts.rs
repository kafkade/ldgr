//! Conflict detection and resolution for cross-device sync.
//!
//! Conflicts occur when the same entity is modified on multiple devices
//! between syncs. Resolution requires user review — no auto-merge for
//! financial data.

use serde::{Deserialize, Serialize};

use super::events::{SyncEvent, VectorClock};

/// A detected sync conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub local_event: SyncEvent,
    pub remote_event: SyncEvent,
    pub detected_at: String,
    pub resolved: bool,
    pub resolution: Option<ConflictResolution>,
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Keep the local version.
    KeepLocal,
    /// Accept the remote version.
    KeepRemote,
    /// Manually merged (new version created).
    Merged,
}

/// Result of merging a remote batch with local state.
#[derive(Debug, Clone, Default)]
pub struct MergeResult {
    /// Events that applied cleanly (no conflict).
    pub applied: Vec<SyncEvent>,
    /// Conflicts requiring user review.
    pub conflicts: Vec<SyncConflict>,
    /// Events skipped (already applied, based on vector clock).
    pub skipped: usize,
}

/// Merge remote events into local state.
///
/// Compares entity IDs: if a remote event touches an entity that also
/// has a local pending event, it's a conflict. Otherwise, it applies
/// cleanly.
pub fn merge_events(
    local_pending: &[SyncEvent],
    remote_events: &[SyncEvent],
    local_clock: &VectorClock,
    remote_clock: &VectorClock,
    current_time: &str,
) -> MergeResult {
    let mut result = MergeResult::default();

    // Index local pending events by entity_id
    let local_entities: std::collections::BTreeMap<&str, &SyncEvent> = local_pending
        .iter()
        .map(|e| (e.entity_id.as_str(), e))
        .collect();

    for remote in remote_events {
        // Skip if we've already seen this event (our clock dominates)
        if local_clock.dominates(remote_clock) {
            result.skipped += 1;
            continue;
        }

        // Check for conflict: same entity modified locally and remotely
        if let Some(&local) = local_entities.get(remote.entity_id.as_str()) {
            result.conflicts.push(SyncConflict {
                id: uuid::Uuid::now_v7().to_string(),
                entity_type: remote.entity_type.as_str().to_string(),
                entity_id: remote.entity_id.clone(),
                local_event: local.clone(),
                remote_event: remote.clone(),
                detected_at: current_time.to_string(),
                resolved: false,
                resolution: None,
            });
        } else {
            result.applied.push(remote.clone());
        }
    }

    result
}

/// Check if two vector clocks indicate a potential conflict.
pub fn clocks_diverged(local: &VectorClock, remote: &VectorClock) -> bool {
    local.is_concurrent(remote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::events::{EntityType, Operation, VectorClock};

    fn make_event(id: &str, device: &str, entity_id: &str, clock: u64) -> SyncEvent {
        SyncEvent {
            id: id.into(),
            device_id: device.into(),
            lamport_clock: clock,
            entity_type: EntityType::Transaction,
            entity_id: entity_id.into(),
            operation: Operation::Update,
            payload: vec![],
            version: 1,
            created_at: "2024-01-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn no_conflict_different_entities() {
        let local = vec![make_event("e1", "dev_a", "txn_1", 1)];
        let remote = vec![make_event("e2", "dev_b", "txn_2", 2)];

        let mut lc = VectorClock::default();
        lc.tick("dev_a");
        let mut rc = VectorClock::default();
        rc.tick("dev_b");

        let result = merge_events(&local, &remote, &lc, &rc, "2024-06-01");
        assert_eq!(result.applied.len(), 1);
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn conflict_same_entity() {
        let local = vec![make_event("e1", "dev_a", "txn_1", 1)];
        let remote = vec![make_event("e2", "dev_b", "txn_1", 2)];

        let mut lc = VectorClock::default();
        lc.tick("dev_a");
        let mut rc = VectorClock::default();
        rc.tick("dev_b");

        let result = merge_events(&local, &remote, &lc, &rc, "2024-06-01");
        assert!(result.applied.is_empty());
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].entity_id, "txn_1");
    }

    #[test]
    fn diverged_clocks() {
        let mut vc1 = VectorClock::default();
        vc1.tick("a");
        let mut vc2 = VectorClock::default();
        vc2.tick("b");
        assert!(clocks_diverged(&vc1, &vc2));
    }

    #[test]
    fn non_diverged_clocks() {
        let mut vc1 = VectorClock::default();
        vc1.tick("a");
        vc1.tick("a");
        let mut vc2 = VectorClock::default();
        vc2.tick("a");
        assert!(!clocks_diverged(&vc1, &vc2)); // vc1 dominates
    }

    #[test]
    fn conflict_resolution_types() {
        let conflict = SyncConflict {
            id: "c1".into(),
            entity_type: "transaction".into(),
            entity_id: "txn_1".into(),
            local_event: make_event("e1", "dev_a", "txn_1", 1),
            remote_event: make_event("e2", "dev_b", "txn_1", 2),
            detected_at: "2024-06-01".into(),
            resolved: true,
            resolution: Some(ConflictResolution::KeepLocal),
        };
        assert!(conflict.resolved);
        assert_eq!(conflict.resolution, Some(ConflictResolution::KeepLocal));
    }
}
