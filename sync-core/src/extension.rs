//! Custom MLS extensions and proposals for the sync protocol.
//!
//! All protocol-specific extensions share a single [`SYNC_EXTENSION_TYPE`],
//! with variant tags encoded in the payload. Similarly, all custom proposals
//! share a single [`SYNC_PROPOSAL_TYPE`] via the [`SyncProposal`] enum.

use std::collections::BTreeMap;

use iroh::EndpointAddr;
use mls_rs::extension::{ExtensionType, MlsCodecExtension};
use mls_rs::group::proposal::MlsCustomProposal;
use mls_rs::mls_rs_codec::{self as mls_rs_codec, MlsDecode, MlsEncode, MlsSize};
use mls_rs_core::group::ProposalType;
use serde::{Deserialize, Serialize};

use crate::proposal::AcceptorId;
use crate::protocol::MemberFingerprint;

/// Single extension type for all sync protocol extensions (private use range).
///
/// The variant is encoded inside the extension data, so different MLS
/// contexts (group context, key package, group info) can each carry their
/// own [`SyncExt`] variant without type-ID collisions.
pub const SYNC_EXTENSION_TYPE: ExtensionType = ExtensionType::new(0xF795);

/// Single proposal type for all sync protocol custom proposals (private use range).
pub const SYNC_PROPOSAL_TYPE: ProposalType = ProposalType::new(0xF796);

// ---------------------------------------------------------------------------
// SyncExt — unified extension enum
// ---------------------------------------------------------------------------

/// Unified extension for all sync protocol data.
///
/// Each MLS context (group context, key package, group info) stores one
/// instance. The variant tag disambiguates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncExt {
    /// Group context: acceptor addresses + CRDT type registration.
    GroupContext {
        acceptors: Vec<EndpointAddr>,
        crdt_type_id: String,
    },
    /// Key package: member's endpoint address + supported CRDT types.
    KeyPackage {
        addr: EndpointAddr,
        supported_crdts: Vec<String>,
    },
    /// Group info (welcome): current acceptor list + optional CRDT snapshot.
    GroupInfo {
        acceptors: Vec<EndpointAddr>,
        snapshot: Option<Vec<u8>>,
    },
}

impl SyncExt {
    #[must_use]
    pub fn group_context(
        acceptors: impl IntoIterator<Item = EndpointAddr>,
        crdt_type_id: impl Into<String>,
    ) -> Self {
        Self::GroupContext {
            acceptors: acceptors.into_iter().collect(),
            crdt_type_id: crdt_type_id.into(),
        }
    }

    #[must_use]
    pub fn key_package(
        addr: EndpointAddr,
        supported_crdts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::KeyPackage {
            addr,
            supported_crdts: supported_crdts.into_iter().map(Into::into).collect(),
        }
    }

    #[must_use]
    pub fn group_info(
        acceptors: impl IntoIterator<Item = EndpointAddr>,
        snapshot: Option<Vec<u8>>,
    ) -> Self {
        Self::GroupInfo {
            acceptors: acceptors.into_iter().collect(),
            snapshot,
        }
    }

    #[must_use]
    pub fn acceptors(&self) -> Option<&[EndpointAddr]> {
        match self {
            Self::GroupContext { acceptors, .. } | Self::GroupInfo { acceptors, .. } => {
                Some(acceptors)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn acceptor_ids(&self) -> Vec<AcceptorId> {
        self.acceptors().map_or_else(Vec::new, |addrs| {
            addrs
                .iter()
                .map(|addr| AcceptorId::from_bytes(*addr.id.as_bytes()))
                .collect()
        })
    }

    #[must_use]
    pub fn crdt_type_id(&self) -> Option<&str> {
        match self {
            Self::GroupContext { crdt_type_id, .. } => Some(crdt_type_id),
            _ => None,
        }
    }

    #[must_use]
    pub fn member_addr(&self) -> Option<&EndpointAddr> {
        match self {
            Self::KeyPackage { addr, .. } => Some(addr),
            _ => None,
        }
    }

    #[must_use]
    pub fn supports_crdt(&self, type_id: &str) -> bool {
        match self {
            Self::KeyPackage {
                supported_crdts, ..
            } => supported_crdts.iter().any(|id| id == type_id),
            _ => false,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Option<&[u8]> {
        match self {
            Self::GroupInfo {
                snapshot: Some(s), ..
            } => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn supported_crdts(&self) -> Option<&[String]> {
        match self {
            Self::KeyPackage {
                supported_crdts, ..
            } => Some(supported_crdts),
            _ => None,
        }
    }
}

impl MlsSize for SyncExt {
    fn mls_encoded_len(&self) -> usize {
        postcard::to_allocvec(self).map_or(4, |v| 4 + v.len())
    }
}

impl MlsEncode for SyncExt {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), mls_rs_codec::Error> {
        postcard_encode(self, writer)
    }
}

impl MlsDecode for SyncExt {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, mls_rs_codec::Error> {
        postcard_decode(reader)
    }
}

impl MlsCodecExtension for SyncExt {
    fn extension_type() -> ExtensionType {
        SYNC_EXTENSION_TYPE
    }
}

// ---------------------------------------------------------------------------
// SyncProposal — unified custom proposal enum
// ---------------------------------------------------------------------------

/// Unified custom proposal for all sync protocol actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncProposal {
    AcceptorAdd(EndpointAddr),
    AcceptorRemove(AcceptorId),
    CompactionClaim {
        level: u8,
        watermark: BTreeMap<MemberFingerprint, u64>,
        deadline: u64,
    },
    CompactionComplete {
        level: u8,
        watermark: BTreeMap<MemberFingerprint, u64>,
    },
}

impl SyncProposal {
    #[must_use]
    pub fn acceptor_add(addr: EndpointAddr) -> Self {
        Self::AcceptorAdd(addr)
    }

    #[must_use]
    pub fn acceptor_remove(id: AcceptorId) -> Self {
        Self::AcceptorRemove(id)
    }

    #[must_use]
    pub fn compaction_claim(
        level: u8,
        watermark: BTreeMap<MemberFingerprint, u64>,
        deadline: u64,
    ) -> Self {
        Self::CompactionClaim {
            level,
            watermark,
            deadline,
        }
    }

    #[must_use]
    pub fn compaction_complete(level: u8, watermark: BTreeMap<MemberFingerprint, u64>) -> Self {
        Self::CompactionComplete { level, watermark }
    }
}

impl MlsSize for SyncProposal {
    fn mls_encoded_len(&self) -> usize {
        postcard::to_allocvec(self).map_or(4, |v| 4 + v.len())
    }
}

impl MlsEncode for SyncProposal {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), mls_rs_codec::Error> {
        postcard_encode(self, writer)
    }
}

impl MlsDecode for SyncProposal {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, mls_rs_codec::Error> {
        postcard_decode(reader)
    }
}

impl MlsCustomProposal for SyncProposal {
    fn proposal_type() -> ProposalType {
        SYNC_PROPOSAL_TYPE
    }
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

const POSTCARD_ERROR: u8 = 1;

#[expect(clippy::cast_possible_truncation)]
fn postcard_encode<T: Serialize>(
    value: &T,
    writer: &mut Vec<u8>,
) -> Result<(), mls_rs_codec::Error> {
    let bytes =
        postcard::to_allocvec(value).map_err(|_| mls_rs_codec::Error::Custom(POSTCARD_ERROR))?;
    let len = bytes.len() as u32;
    writer.extend_from_slice(&len.to_be_bytes());
    writer.extend_from_slice(&bytes);
    Ok(())
}

fn postcard_decode<T: for<'de> Deserialize<'de>>(
    reader: &mut &[u8],
) -> Result<T, mls_rs_codec::Error> {
    let data = read_length_prefixed(reader)?;
    postcard::from_bytes(data).map_err(|_| mls_rs_codec::Error::Custom(POSTCARD_ERROR))
}

fn read_length_prefixed<'a>(reader: &mut &'a [u8]) -> Result<&'a [u8], mls_rs_codec::Error> {
    if reader.len() < 4 {
        return Err(mls_rs_codec::Error::UnexpectedEOF);
    }
    let len = u32::from_be_bytes([reader[0], reader[1], reader[2], reader[3]]) as usize;
    *reader = &reader[4..];

    if reader.len() < len {
        return Err(mls_rs_codec::Error::UnexpectedEOF);
    }
    let data = &reader[..len];
    *reader = &reader[len..];
    Ok(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    fn test_addr(seed: u8) -> EndpointAddr {
        let secret = SecretKey::from_bytes(&[seed; 32]);
        EndpointAddr::new(secret.public())
    }

    #[test]
    fn sync_ext_group_context_roundtrip() {
        let addrs = vec![test_addr(1), test_addr(2), test_addr(3)];
        let ext = SyncExt::group_context(addrs, "yjs");

        let encoded = ext.mls_encode_to_vec().unwrap();
        let decoded = SyncExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.acceptors().unwrap().len(), 3);
        assert_eq!(decoded.crdt_type_id().unwrap(), "yjs");
    }

    #[test]
    fn sync_ext_group_context_empty_acceptors() {
        let ext = SyncExt::group_context(Vec::<EndpointAddr>::new(), "none");

        let encoded = ext.mls_encode_to_vec().unwrap();
        let decoded = SyncExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert!(decoded.acceptors().unwrap().is_empty());
    }

    #[test]
    fn sync_ext_key_package_roundtrip() {
        let addr = test_addr(42);
        let ext = SyncExt::key_package(addr.clone(), ["yjs", "automerge"]);

        let encoded = ext.mls_encode_to_vec().unwrap();
        let decoded = SyncExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.member_addr().unwrap(), &addr);
        assert!(decoded.supports_crdt("yjs"));
        assert!(decoded.supports_crdt("automerge"));
        assert!(!decoded.supports_crdt("unknown"));
    }

    #[test]
    fn sync_ext_group_info_roundtrip() {
        let addrs = vec![test_addr(1), test_addr(2)];
        let ext = SyncExt::group_info(addrs, Some(b"crdt snapshot data".to_vec()));

        let encoded = ext.mls_encode_to_vec().unwrap();
        let decoded = SyncExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.snapshot().unwrap(), b"crdt snapshot data");
        assert_eq!(decoded.acceptors().unwrap().len(), 2);
    }

    #[test]
    fn sync_ext_group_info_no_snapshot() {
        let ext = SyncExt::group_info(Vec::<EndpointAddr>::new(), None);

        let encoded = ext.mls_encode_to_vec().unwrap();
        let decoded = SyncExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert!(decoded.snapshot().is_none());
    }

    #[test]
    fn sync_ext_acceptor_ids() {
        let addr1 = test_addr(1);
        let addr2 = test_addr(2);
        let expected_id1 = AcceptorId::from_bytes(*addr1.id.as_bytes());
        let expected_id2 = AcceptorId::from_bytes(*addr2.id.as_bytes());

        let ext = SyncExt::group_context(vec![addr1, addr2], "none");
        let ids = ext.acceptor_ids();

        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], expected_id1);
        assert_eq!(ids[1], expected_id2);
    }

    #[test]
    fn sync_proposal_acceptor_add_roundtrip() {
        let addr = test_addr(42);
        let proposal = SyncProposal::acceptor_add(addr.clone());

        let custom = proposal.to_custom_proposal().unwrap();
        assert_eq!(custom.proposal_type(), SYNC_PROPOSAL_TYPE);

        let decoded = SyncProposal::from_custom_proposal(&custom).unwrap();
        assert_eq!(decoded, proposal);
        assert!(matches!(decoded, SyncProposal::AcceptorAdd(a) if a == addr));
    }

    #[test]
    fn sync_proposal_acceptor_remove_roundtrip() {
        let id = AcceptorId::from_bytes([42u8; 32]);
        let proposal = SyncProposal::acceptor_remove(id);

        let custom = proposal.to_custom_proposal().unwrap();
        let decoded = SyncProposal::from_custom_proposal(&custom).unwrap();
        assert_eq!(decoded, proposal);
    }

    #[test]
    fn sync_proposal_compaction_claim_roundtrip() {
        let mut watermark = BTreeMap::new();
        watermark.insert(MemberFingerprint([1u8; 32]), 42);
        watermark.insert(MemberFingerprint([2u8; 32]), 100);
        let proposal = SyncProposal::compaction_claim(1, watermark, 1_700_000_000);

        let custom = proposal.to_custom_proposal().unwrap();
        let decoded = SyncProposal::from_custom_proposal(&custom).unwrap();
        assert_eq!(decoded, proposal);
    }

    #[test]
    fn sync_proposal_compaction_complete_roundtrip() {
        let mut watermark = BTreeMap::new();
        watermark.insert(MemberFingerprint([3u8; 32]), 99);
        let proposal = SyncProposal::compaction_complete(2, watermark);

        let custom = proposal.to_custom_proposal().unwrap();
        let decoded = SyncProposal::from_custom_proposal(&custom).unwrap();
        assert_eq!(decoded, proposal);
    }

    #[test]
    fn sync_types_in_private_range() {
        assert!(SYNC_EXTENSION_TYPE.raw_value() >= 0xF000);
        assert!(SYNC_PROPOSAL_TYPE.raw_value() >= 0xF000);
    }
}
