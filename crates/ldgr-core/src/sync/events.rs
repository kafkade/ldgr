//! Sync event generation, Lamport clock, vector clocks, and batch operations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Entity types that can be synced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    Transaction,
    Account,
    Price,
    Budget,
    Goal,
}

impl EntityType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transaction => "transaction",
            Self::Account => "account",
            Self::Price => "price",
            Self::Budget => "budget",
            Self::Goal => "goal",
        }
    }
}

/// Sync operation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

/// A sync event capturing a single entity mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEvent {
    pub id: String,
    pub device_id: String,
    pub lamport_clock: u64,
    pub entity_type: EntityType,
    pub entity_id: String,
    pub operation: Operation,
    /// Full entity state as JSON (pre-encryption).
    pub payload: Vec<u8>,
    pub version: u32,
    pub created_at: String,
}

/// Lamport logical clock for event ordering.
#[derive(Debug, Clone, Default)]
pub struct LamportClock {
    value: u64,
}

impl LamportClock {
    pub fn new(initial: u64) -> Self {
        Self { value: initial }
    }

    /// Tick the clock for a local event. Returns the new value.
    pub fn tick(&mut self) -> u64 {
        self.value += 1;
        self.value
    }

    /// Update from a received remote clock value.
    pub fn receive(&mut self, remote: u64) {
        self.value = self.value.max(remote) + 1;
    }

    pub fn current(&self) -> u64 {
        self.value
    }
}

/// Vector clock tracking per-device logical time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorClock {
    pub clocks: BTreeMap<String, u64>,
}

impl VectorClock {
    /// Increment the clock for a device.
    pub fn tick(&mut self, device_id: &str) -> u64 {
        let entry = self.clocks.entry(device_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Merge a remote vector clock into this one (element-wise max).
    pub fn merge(&mut self, other: &VectorClock) {
        for (device, &clock) in &other.clocks {
            let entry = self.clocks.entry(device.clone()).or_insert(0);
            *entry = (*entry).max(clock);
        }
    }

    /// Check if this clock dominates (is ≥ for all devices) the other.
    pub fn dominates(&self, other: &VectorClock) -> bool {
        other
            .clocks
            .iter()
            .all(|(device, &clock)| self.clocks.get(device).copied().unwrap_or(0) >= clock)
    }

    /// Check if two clocks are concurrent (neither dominates the other).
    pub fn is_concurrent(&self, other: &VectorClock) -> bool {
        !self.dominates(other) && !other.dominates(self)
    }
}

/// A batch of events serialized for encryption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBatch {
    pub device_id: String,
    pub events: Vec<SyncEvent>,
    pub vector_clock: VectorClock,
}

/// Create an event batch from a list of events.
pub fn create_batch(device_id: &str, events: Vec<SyncEvent>, clock: &VectorClock) -> EventBatch {
    EventBatch {
        device_id: device_id.to_string(),
        events,
        vector_clock: clock.clone(),
    }
}

/// Serialize a batch to bytes (for encryption).
pub fn serialize_batch(batch: &EventBatch) -> Result<Vec<u8>, String> {
    serde_json::to_vec(batch).map_err(|e| format!("failed to serialize batch: {e}"))
}

/// Deserialize a batch from bytes (after decryption).
pub fn deserialize_batch(data: &[u8]) -> Result<EventBatch, String> {
    serde_json::from_slice(data).map_err(|e| format!("failed to deserialize batch: {e}"))
}

/// Deterministic total order for events: Lamport clock → event ID → device ID.
pub fn total_order(a: &SyncEvent, b: &SyncEvent) -> std::cmp::Ordering {
    a.lamport_clock
        .cmp(&b.lamport_clock)
        .then_with(|| a.id.cmp(&b.id))
        .then_with(|| a.device_id.cmp(&b.device_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamport_tick() {
        let mut clock = LamportClock::new(0);
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
        assert_eq!(clock.current(), 2);
    }

    #[test]
    fn lamport_receive() {
        let mut clock = LamportClock::new(5);
        clock.receive(10);
        assert_eq!(clock.current(), 11);
        clock.receive(3);
        assert_eq!(clock.current(), 12); // max(11, 3) + 1
    }

    #[test]
    fn vector_clock_tick() {
        let mut vc = VectorClock::default();
        assert_eq!(vc.tick("device_a"), 1);
        assert_eq!(vc.tick("device_a"), 2);
        assert_eq!(vc.tick("device_b"), 1);
    }

    #[test]
    fn vector_clock_merge() {
        let mut vc1 = VectorClock::default();
        vc1.tick("a");
        vc1.tick("a"); // a=2

        let mut vc2 = VectorClock::default();
        vc2.tick("a"); // a=1
        vc2.tick("b"); // b=1

        vc1.merge(&vc2);
        assert_eq!(vc1.clocks["a"], 2); // max(2, 1)
        assert_eq!(vc1.clocks["b"], 1); // max(0, 1)
    }

    #[test]
    fn vector_clock_dominance() {
        let mut vc1 = VectorClock::default();
        vc1.tick("a");
        vc1.tick("a");

        let mut vc2 = VectorClock::default();
        vc2.tick("a");

        assert!(vc1.dominates(&vc2));
        assert!(!vc2.dominates(&vc1));
    }

    #[test]
    fn vector_clock_concurrent() {
        let mut vc1 = VectorClock::default();
        vc1.tick("a");
        vc1.tick("a"); // a=2, b=0

        let mut vc2 = VectorClock::default();
        vc2.tick("a"); // a=1
        vc2.tick("b"); // b=1

        assert!(vc1.is_concurrent(&vc2));
    }

    #[test]
    fn batch_serialize_round_trip() {
        let event = SyncEvent {
            id: "evt1".into(),
            device_id: "dev1".into(),
            lamport_clock: 1,
            entity_type: EntityType::Transaction,
            entity_id: "txn1".into(),
            operation: Operation::Create,
            payload: b"test payload".to_vec(),
            version: 1,
            created_at: "2024-01-15T00:00:00Z".into(),
        };

        let clock = VectorClock::default();
        let batch = create_batch("dev1", vec![event], &clock);
        let bytes = serialize_batch(&batch).unwrap();
        let restored = deserialize_batch(&bytes).unwrap();

        assert_eq!(restored.events.len(), 1);
        assert_eq!(restored.events[0].entity_id, "txn1");
    }

    #[test]
    fn total_order_by_lamport() {
        let e1 = SyncEvent {
            id: "b".into(),
            device_id: "d1".into(),
            lamport_clock: 1,
            entity_type: EntityType::Account,
            entity_id: "a".into(),
            operation: Operation::Create,
            payload: vec![],
            version: 1,
            created_at: String::new(),
        };
        let e2 = SyncEvent {
            id: "a".into(),
            device_id: "d2".into(),
            lamport_clock: 2,
            entity_type: EntityType::Account,
            entity_id: "a".into(),
            operation: Operation::Update,
            payload: vec![],
            version: 1,
            created_at: String::new(),
        };
        assert_eq!(total_order(&e1, &e2), std::cmp::Ordering::Less);
    }
}
