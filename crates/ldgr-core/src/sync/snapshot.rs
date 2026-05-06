//! Snapshot/compaction for efficient device onboarding.
//!
//! Snapshots capture the full materialized state at a point in the event
//! log, enabling new devices to bootstrap without replaying the entire
//! event history.

use serde::{Deserialize, Serialize};

use super::events::VectorClock;

/// A snapshot of the full vault state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    /// Number of events included up to this snapshot.
    pub event_count: u64,
    /// Vector clock at snapshot time.
    pub vector_clock: VectorClock,
    /// Serialized state (all entities as JSON, pre-encryption).
    pub payload: Vec<u8>,
    pub created_at: String,
}

/// Policy for when to create snapshots.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    /// Create snapshot after this many new events.
    pub event_threshold: u64,
    /// Maximum number of snapshots to retain.
    pub max_snapshots: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            event_threshold: 1000,
            max_snapshots: 3,
        }
    }
}

/// Check if a new snapshot should be created.
pub fn should_compact(events_since_last_snapshot: u64, policy: &CompactionPolicy) -> bool {
    events_since_last_snapshot >= policy.event_threshold
}

/// Determine which old snapshots to prune, keeping `max_snapshots` newest.
///
/// Returns the IDs of snapshots to delete.
pub fn snapshots_to_prune(snapshot_ids: &[String], policy: &CompactionPolicy) -> Vec<String> {
    if snapshot_ids.len() <= policy.max_snapshots {
        return Vec::new();
    }
    let prune_count = snapshot_ids.len() - policy.max_snapshots;
    snapshot_ids[..prune_count].to_vec()
}

/// Onboarding plan: what a new device needs to download.
#[derive(Debug, Clone)]
pub struct OnboardingPlan {
    /// The latest snapshot to download (if available).
    pub snapshot_id: Option<String>,
    /// Event batch IDs to download after the snapshot.
    pub event_batch_ids: Vec<String>,
    /// Total estimated size in bytes.
    pub estimated_bytes: u64,
}

/// Compute what a new device needs for onboarding.
///
/// If a snapshot exists, start from there + subsequent event batches.
/// Otherwise, download all event batches from the beginning.
#[allow(clippy::cast_possible_truncation)]
pub fn plan_onboarding(
    latest_snapshot: Option<&Snapshot>,
    total_event_batches: &[String],
    snapshot_event_count: u64,
    batch_sizes: &[u64],
) -> OnboardingPlan {
    let sc = snapshot_event_count as usize;
    if let Some(snap) = latest_snapshot {
        let batches_needed = if sc < total_event_batches.len() {
            total_event_batches[sc..].to_vec()
        } else {
            Vec::new()
        };
        let estimated: u64 = batch_sizes.iter().skip(sc).sum();
        OnboardingPlan {
            snapshot_id: Some(snap.id.clone()),
            event_batch_ids: batches_needed,
            estimated_bytes: snap.payload.len() as u64 + estimated,
        }
    } else {
        let estimated: u64 = batch_sizes.iter().sum();
        OnboardingPlan {
            snapshot_id: None,
            event_batch_ids: total_event_batches.to_vec(),
            estimated_bytes: estimated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_compact_at_threshold() {
        let policy = CompactionPolicy::default();
        assert!(!should_compact(999, &policy));
        assert!(should_compact(1000, &policy));
        assert!(should_compact(1500, &policy));
    }

    #[test]
    fn prune_old_snapshots() {
        let policy = CompactionPolicy {
            max_snapshots: 3,
            ..Default::default()
        };
        let ids: Vec<String> = (1..=5).map(|i| format!("snap_{i}")).collect();
        let to_prune = snapshots_to_prune(&ids, &policy);
        assert_eq!(to_prune, vec!["snap_1", "snap_2"]);
    }

    #[test]
    fn prune_nothing_when_under_limit() {
        let policy = CompactionPolicy {
            max_snapshots: 5,
            ..Default::default()
        };
        let ids: Vec<String> = (1..=3).map(|i| format!("snap_{i}")).collect();
        assert!(snapshots_to_prune(&ids, &policy).is_empty());
    }

    #[test]
    fn onboarding_with_snapshot() {
        let snap = Snapshot {
            id: "snap_1".into(),
            event_count: 1000,
            vector_clock: VectorClock::default(),
            payload: vec![0; 5000],
            created_at: "2024-06-01".into(),
        };
        let batches: Vec<String> = (0..1200).map(|i| format!("batch_{i}")).collect();
        let sizes: Vec<u64> = vec![100; 1200];

        let plan = plan_onboarding(Some(&snap), &batches, 1000, &sizes);
        assert_eq!(plan.snapshot_id.as_deref(), Some("snap_1"));
        assert_eq!(plan.event_batch_ids.len(), 200); // only post-snapshot
    }

    #[test]
    fn onboarding_without_snapshot() {
        let batches: Vec<String> = (0..500).map(|i| format!("batch_{i}")).collect();
        let sizes: Vec<u64> = vec![100; 500];

        let plan = plan_onboarding(None, &batches, 0, &sizes);
        assert!(plan.snapshot_id.is_none());
        assert_eq!(plan.event_batch_ids.len(), 500);
        assert_eq!(plan.estimated_bytes, 50000);
    }
}
