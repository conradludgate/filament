//! Yrs (Yjs) CRDT implementation — wraps `yrs::Doc` for the [`Crdt`] trait.

use error_stack::{Report, ResultExt};
use universal_sync_core::{Crdt, CrdtError, CrdtFactory};
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

pub struct YrsCrdt {
    doc: Doc,
    /// For computing diffs since the last flush
    last_flushed_sv: StateVector,
}

impl YrsCrdt {
    #[must_use]
    pub fn new() -> Self {
        let doc = Doc::new();
        let last_flushed_sv = doc.transact().state_vector();
        Self {
            doc,
            last_flushed_sv,
        }
    }

    /// Each member in a group should have a unique client ID.
    #[must_use]
    pub fn with_client_id(client_id: u64) -> Self {
        let doc = Doc::with_client_id(client_id);
        let last_flushed_sv = doc.transact().state_vector();
        Self {
            doc,
            last_flushed_sv,
        }
    }

    #[must_use]
    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    #[must_use]
    pub fn state_vector(&self) -> StateVector {
        self.doc.transact().state_vector()
    }

    pub fn encode_diff(&self, sv: &StateVector) -> Result<Vec<u8>, Report<CrdtError>> {
        let txn = self.doc.transact();
        Ok(txn.encode_diff_v2(sv))
    }
}

impl Default for YrsCrdt {
    fn default() -> Self {
        Self::new()
    }
}

impl Crdt for YrsCrdt {
    fn protocol_name(&self) -> &str {
        "yrs"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }

    fn apply(&mut self, operation: &[u8]) -> Result<(), Report<CrdtError>> {
        let update = Update::decode_v2(operation).change_context(CrdtError)?;

        self.doc
            .transact_mut()
            .apply_update(update)
            .change_context(CrdtError)?;

        Ok(())
    }

    fn merge(&mut self, snapshot: &[u8]) -> Result<(), Report<CrdtError>> {
        self.apply(snapshot)
    }

    fn snapshot(&self) -> Result<Vec<u8>, Report<CrdtError>> {
        let txn = self.doc.transact();
        Ok(txn.encode_state_as_update_v2(&StateVector::default()))
    }

    fn flush_update(&mut self) -> Result<Option<Vec<u8>>, Report<CrdtError>> {
        let txn = self.doc.transact();
        let current_sv = txn.state_vector();
        if current_sv == self.last_flushed_sv {
            return Ok(None);
        }
        let update = txn.encode_diff_v2(&self.last_flushed_sv);
        drop(txn);
        self.last_flushed_sv = current_sv;
        Ok(Some(update))
    }
}

#[derive(Debug, Default, Clone)]
pub struct YrsCrdtFactory;

impl YrsCrdtFactory {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl CrdtFactory for YrsCrdtFactory {
    fn type_id(&self) -> &str {
        "yrs"
    }

    fn create(&self, client_id: u64) -> Box<dyn Crdt> {
        Box::new(YrsCrdt::with_client_id(client_id))
    }

    fn from_snapshot(
        &self,
        snapshot: &[u8],
        client_id: u64,
    ) -> Result<Box<dyn Crdt>, Report<CrdtError>> {
        let mut crdt = YrsCrdt::with_client_id(client_id);
        crdt.merge(snapshot)?;
        Ok(Box::new(crdt))
    }
}

#[cfg(test)]
mod tests {
    use yrs::types::ToJson;
    use yrs::{Any, GetString, Map, Text, Transact};

    use super::*;

    #[test]
    fn test_yrs_crdt_basic() {
        let crdt = YrsCrdt::new();
        assert_eq!(Crdt::protocol_name(&crdt), "yrs");

        let snapshot = crdt.snapshot().unwrap();
        assert!(!snapshot.is_empty());
    }

    #[test]
    fn test_yrs_crdt_text() {
        let mut crdt1 = YrsCrdt::with_client_id(1);
        let mut crdt2 = YrsCrdt::with_client_id(2);

        {
            let text = crdt1.doc().get_or_insert_text("my-text");
            let mut txn = crdt1.doc().transact_mut();
            text.insert(&mut txn, 0, "Hello, ");
        }

        let update = crdt1.snapshot().unwrap();
        crdt2.merge(&update).unwrap();

        {
            let text = crdt2.doc().get_or_insert_text("my-text");
            let txn = crdt2.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello, ");
        }

        {
            let text = crdt2.doc().get_or_insert_text("my-text");
            let mut txn = crdt2.doc().transact_mut();
            text.insert(&mut txn, 7, "World!");
        }

        let update2 = crdt2.snapshot().unwrap();
        crdt1.merge(&update2).unwrap();

        {
            let text = crdt1.doc().get_or_insert_text("my-text");
            let txn = crdt1.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello, World!");
        }
    }

    #[test]
    fn test_yrs_crdt_map() {
        let crdt = YrsCrdt::with_client_id(1);

        {
            let map = crdt.doc().get_or_insert_map("my-map");
            let mut txn = crdt.doc().transact_mut();
            map.insert(&mut txn, "key1", "value1");
            map.insert(&mut txn, "key2", 42i64);
        }

        let snapshot = crdt.snapshot().unwrap();
        let mut crdt2 = YrsCrdt::with_client_id(2);
        crdt2.merge(&snapshot).unwrap();

        {
            let map = crdt2.doc().get_or_insert_map("my-map");
            let txn = crdt2.doc().transact();
            let json = map.to_json(&txn);

            if let Any::Map(m) = json {
                assert_eq!(m.get("key1"), Some(&Any::String("value1".into())));
                assert_eq!(m.get("key2"), Some(&Any::Number(42.0)));
            } else {
                panic!("Expected map");
            }
        }
    }

    #[test]
    fn test_yrs_factory() {
        let factory = YrsCrdtFactory::new();
        assert_eq!(CrdtFactory::type_id(&factory), "yrs");

        let crdt = factory.create(1);
        assert_eq!(Crdt::protocol_name(&*crdt), "yrs");

        let snapshot = crdt.snapshot().unwrap();
        let crdt2 = factory.from_snapshot(&snapshot, 2).unwrap();
        assert_eq!(Crdt::protocol_name(&*crdt2), "yrs");
    }

    #[test]
    fn test_compact_multiple_updates() {
        let factory = YrsCrdtFactory::new();
        let mut crdt = factory.create(1);

        let mut updates = Vec::new();
        let insert_and_flush = |crdt: &mut Box<dyn Crdt>, pos: u32, text: &str| {
            let yrs = crdt.as_any_mut().downcast_mut::<YrsCrdt>().unwrap();
            let t = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            t.insert(&mut txn, pos, text);
            drop(txn);
            crdt.flush_update().unwrap().unwrap()
        };

        updates.push(insert_and_flush(&mut crdt, 0, "aaa"));
        updates.push(insert_and_flush(&mut crdt, 3, "bbb"));
        updates.push(insert_and_flush(&mut crdt, 6, "ccc"));

        let refs: Vec<&[u8]> = updates.iter().map(|u| u.as_slice()).collect();
        let compacted = factory.compact(None, &refs).unwrap();

        let fresh = factory.from_snapshot(&compacted, 99).unwrap();
        let yrs = fresh.as_any().downcast_ref::<YrsCrdt>().unwrap();
        let text = yrs.doc().get_or_insert_text("doc");
        let txn = yrs.doc().transact();
        assert_eq!(text.get_string(&txn), "aaabbbccc");
    }

    #[test]
    fn test_compact_with_base_snapshot() {
        let factory = YrsCrdtFactory::new();
        let mut crdt = factory.create(1);

        // Create base content
        {
            let yrs = crdt.as_any_mut().downcast_mut::<YrsCrdt>().unwrap();
            let t = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            t.insert(&mut txn, 0, "base");
        }
        let base = crdt.snapshot().unwrap();

        // Create an incremental update
        {
            let yrs = crdt.as_any_mut().downcast_mut::<YrsCrdt>().unwrap();
            let t = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            t.insert(&mut txn, 4, "+inc");
        }
        let inc = crdt.flush_update().unwrap().unwrap();

        let compacted = factory.compact(Some(&base), &[&inc]).unwrap();

        let fresh = factory.from_snapshot(&compacted, 99).unwrap();
        let yrs = fresh.as_any().downcast_ref::<YrsCrdt>().unwrap();
        let text = yrs.doc().get_or_insert_text("doc");
        let txn = yrs.doc().transact();
        assert_eq!(text.get_string(&txn), "base+inc");
    }

    #[test]
    fn test_compact_empty_updates() {
        let factory = YrsCrdtFactory::new();
        let compacted = factory.compact(None, &[]).unwrap();
        let _fresh = factory.from_snapshot(&compacted, 0).unwrap();
    }

    #[test]
    fn test_from_snapshot_empty_bytes_fails() {
        let factory = YrsCrdtFactory::new();
        assert!(
            factory.from_snapshot(&[], 0).is_err(),
            "empty bytes should fail to decode as a yrs update"
        );
    }

    #[test]
    fn test_concurrent_edits() {
        let mut crdt1 = YrsCrdt::with_client_id(1);
        let mut crdt2 = YrsCrdt::with_client_id(2);

        {
            let text = crdt1.doc().get_or_insert_text("doc");
            let mut txn = crdt1.doc().transact_mut();
            text.insert(&mut txn, 0, "ABC");
        }
        let initial = crdt1.snapshot().unwrap();
        crdt2.merge(&initial).unwrap();

        {
            let text = crdt1.doc().get_or_insert_text("doc");
            let mut txn = crdt1.doc().transact_mut();
            text.insert(&mut txn, 1, "X");
        }

        {
            let text = crdt2.doc().get_or_insert_text("doc");
            let mut txn = crdt2.doc().transact_mut();
            text.insert(&mut txn, 2, "Y");
        }

        let update1 = crdt1.snapshot().unwrap();
        let update2 = crdt2.snapshot().unwrap();
        crdt1.merge(&update2).unwrap();
        crdt2.merge(&update1).unwrap();

        // Both should converge
        let text1 = {
            let text = crdt1.doc().get_or_insert_text("doc");
            let txn = crdt1.doc().transact();
            text.get_string(&txn)
        };
        let text2 = {
            let text = crdt2.doc().get_or_insert_text("doc");
            let txn = crdt2.doc().transact();
            text.get_string(&txn)
        };

        assert_eq!(text1, text2);
        assert!(text1.contains('A'));
        assert!(text1.contains('B'));
        assert!(text1.contains('C'));
        assert!(text1.contains('X'));
        assert!(text1.contains('Y'));
    }

    #[test]
    fn test_echo_idempotent() {
        let mut alice = YrsCrdt::with_client_id(12345);

        // Alice inserts "Hello"
        {
            let text = alice.doc().get_or_insert_text("doc");
            let mut txn = alice.doc().transact_mut();
            text.insert(&mut txn, 0, "Hello");
        }

        // flush_update encodes a diff
        let diff = alice.flush_update().unwrap().unwrap();

        // Echo: apply the same diff back to Alice
        alice.apply(&diff).unwrap();

        {
            let text = alice.doc().get_or_insert_text("doc");
            let txn = alice.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello", "echo should be idempotent");
        }
    }

    #[test]
    fn test_two_peer_with_deterministic_ids() {
        let factory = YrsCrdtFactory::new();
        let alice_id: u64 = 111;
        let bob_id: u64 = 222;

        let mut alice_crdt = factory.create(alice_id);
        let mut bob_crdt = factory.create(bob_id);

        // Alice inserts "Hello"
        {
            let yrs = alice_crdt.as_any_mut().downcast_mut::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            text.insert(&mut txn, 0, "Hello");
        }
        let alice_update = alice_crdt.flush_update().unwrap().unwrap();

        // Bob applies Alice's update
        bob_crdt.apply(&alice_update).unwrap();
        {
            let yrs = bob_crdt.as_any().downcast_ref::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let txn = yrs.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello");
        }

        // Alice receives echo
        alice_crdt.apply(&alice_update).unwrap();
        {
            let yrs = alice_crdt.as_any().downcast_ref::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let txn = yrs.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello", "echo should not duplicate");
        }

        // Bob inserts " World"
        {
            let yrs = bob_crdt.as_any_mut().downcast_mut::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let mut txn = yrs.doc().transact_mut();
            text.insert(&mut txn, 5, " World");
        }
        let bob_update = bob_crdt.flush_update().unwrap().unwrap();

        // Alice applies Bob's update
        alice_crdt.apply(&bob_update).unwrap();
        {
            let yrs = alice_crdt.as_any().downcast_ref::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let txn = yrs.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello World");
        }

        // Alice also applies echo + bob in opposite order (all at once)
        let mut alice2 = factory.create(alice_id);
        alice2.apply(&bob_update).unwrap();
        alice2.apply(&alice_update).unwrap();
        {
            let yrs = alice2.as_any().downcast_ref::<YrsCrdt>().unwrap();
            let text = yrs.doc().get_or_insert_text("doc");
            let txn = yrs.doc().transact();
            assert_eq!(text.get_string(&txn), "Hello World");
        }
    }
}
