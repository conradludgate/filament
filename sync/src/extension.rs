//! MLS extensions for Universal Sync
//!
//! Custom MLS group context extensions used by the sync protocol.

use mls_rs::extension::{ExtensionType, MlsCodecExtension};
use mls_rs::mls_rs_codec::{self as mls_rs_codec, MlsDecode, MlsEncode, MlsSize};

use crate::proposal::AcceptorId;

/// Extension type for the full acceptor list (group context extension)
pub const ACCEPTORS_EXTENSION_TYPE: ExtensionType = ExtensionType::new(0xF795);

/// Extension type for adding an acceptor (private use range: 0xF000-0xFFFF)
pub const ACCEPTOR_ADD_EXTENSION_TYPE: ExtensionType = ExtensionType::new(0xF796);

/// Extension type for removing an acceptor (private use range: 0xF000-0xFFFF)
pub const ACCEPTOR_REMOVE_EXTENSION_TYPE: ExtensionType = ExtensionType::new(0xF797);

/// MLS group context extension containing the full list of acceptors
///
/// This extension is set when the group is created and updated whenever
/// acceptors are added or removed. New members joining via Welcome can
/// read this extension to discover the acceptor set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AcceptorsExt(pub Vec<AcceptorId>);

impl AcceptorsExt {
    /// Create a new AcceptorsExt from a list of acceptor IDs
    pub fn new(acceptors: impl IntoIterator<Item = AcceptorId>) -> Self {
        Self(acceptors.into_iter().collect())
    }

    /// Get the list of acceptor IDs
    pub fn acceptors(&self) -> &[AcceptorId] {
        &self.0
    }
}

impl MlsSize for AcceptorsExt {
    fn mls_encoded_len(&self) -> usize {
        // 4 bytes for length prefix + 32 bytes per acceptor
        4 + self.0.len() * 32
    }
}

impl MlsEncode for AcceptorsExt {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), mls_rs_codec::Error> {
        // Write length as u32
        let len = self.0.len() as u32;
        writer.extend_from_slice(&len.to_be_bytes());
        // Write each acceptor ID
        for acceptor in &self.0 {
            writer.extend_from_slice(&acceptor.0);
        }
        Ok(())
    }
}

impl MlsDecode for AcceptorsExt {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, mls_rs_codec::Error> {
        if reader.len() < 4 {
            return Err(mls_rs_codec::Error::UnexpectedEOF);
        }
        let len = u32::from_be_bytes([reader[0], reader[1], reader[2], reader[3]]) as usize;
        *reader = &reader[4..];

        if reader.len() < len * 32 {
            return Err(mls_rs_codec::Error::UnexpectedEOF);
        }

        let mut acceptors = Vec::with_capacity(len);
        for _ in 0..len {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&reader[..32]);
            *reader = &reader[32..];
            acceptors.push(AcceptorId(bytes));
        }

        Ok(Self(acceptors))
    }
}

impl MlsCodecExtension for AcceptorsExt {
    fn extension_type() -> ExtensionType {
        ACCEPTORS_EXTENSION_TYPE
    }
}

/// MLS group context extension to add a federated acceptor
///
/// When this extension is present in the group context, the acceptor
/// should be added to the set of known acceptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptorAdd(pub AcceptorId);

impl AcceptorAdd {
    /// Create a new AcceptorAdd extension
    pub fn new(acceptor_id: AcceptorId) -> Self {
        Self(acceptor_id)
    }

    /// Get the acceptor ID being added
    pub fn acceptor_id(&self) -> AcceptorId {
        self.0
    }
}

impl MlsSize for AcceptorAdd {
    fn mls_encoded_len(&self) -> usize {
        32 // AcceptorId is always 32 bytes
    }
}

impl MlsEncode for AcceptorAdd {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), mls_rs_codec::Error> {
        writer.extend_from_slice(&self.0.0);
        Ok(())
    }
}

impl MlsDecode for AcceptorAdd {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, mls_rs_codec::Error> {
        if reader.len() < 32 {
            return Err(mls_rs_codec::Error::UnexpectedEOF);
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&reader[..32]);
        *reader = &reader[32..];
        Ok(Self(AcceptorId(bytes)))
    }
}

impl MlsCodecExtension for AcceptorAdd {
    fn extension_type() -> ExtensionType {
        ACCEPTOR_ADD_EXTENSION_TYPE
    }
}

/// MLS group context extension to remove a federated acceptor
///
/// When this extension is present in the group context, the acceptor
/// should be removed from the set of known acceptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptorRemove(pub AcceptorId);

impl AcceptorRemove {
    /// Create a new AcceptorRemove extension
    pub fn new(acceptor_id: AcceptorId) -> Self {
        Self(acceptor_id)
    }

    /// Get the acceptor ID being removed
    pub fn acceptor_id(&self) -> AcceptorId {
        self.0
    }
}

impl MlsSize for AcceptorRemove {
    fn mls_encoded_len(&self) -> usize {
        32 // AcceptorId is always 32 bytes
    }
}

impl MlsEncode for AcceptorRemove {
    fn mls_encode(&self, writer: &mut Vec<u8>) -> Result<(), mls_rs_codec::Error> {
        writer.extend_from_slice(&self.0.0);
        Ok(())
    }
}

impl MlsDecode for AcceptorRemove {
    fn mls_decode(reader: &mut &[u8]) -> Result<Self, mls_rs_codec::Error> {
        if reader.len() < 32 {
            return Err(mls_rs_codec::Error::UnexpectedEOF);
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&reader[..32]);
        *reader = &reader[32..];
        Ok(Self(AcceptorId(bytes)))
    }
}

impl MlsCodecExtension for AcceptorRemove {
    fn extension_type() -> ExtensionType {
        ACCEPTOR_REMOVE_EXTENSION_TYPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acceptors_ext_roundtrip() {
        let ids = vec![
            AcceptorId::from_bytes([1u8; 32]),
            AcceptorId::from_bytes([2u8; 32]),
            AcceptorId::from_bytes([3u8; 32]),
        ];
        let ext = AcceptorsExt::new(ids.clone());

        let encoded = ext.mls_encode_to_vec().unwrap();
        // 4 bytes for length + 3 * 32 bytes for acceptors
        assert_eq!(encoded.len(), 4 + 3 * 32);

        let decoded = AcceptorsExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.acceptors(), &ids);
    }

    #[test]
    fn test_acceptors_ext_empty() {
        let ext = AcceptorsExt::default();
        assert!(ext.acceptors().is_empty());

        let encoded = ext.mls_encode_to_vec().unwrap();
        assert_eq!(encoded.len(), 4); // Just the length prefix

        let decoded = AcceptorsExt::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
    }

    #[test]
    fn test_acceptor_add_roundtrip() {
        let id = AcceptorId::from_bytes([42u8; 32]);
        let ext = AcceptorAdd::new(id);

        let encoded = ext.mls_encode_to_vec().unwrap();
        assert_eq!(encoded.len(), 32);

        let decoded = AcceptorAdd::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.acceptor_id(), id);
    }

    #[test]
    fn test_acceptor_remove_roundtrip() {
        let id = AcceptorId::from_bytes([42u8; 32]);
        let ext = AcceptorRemove::new(id);

        let encoded = ext.mls_encode_to_vec().unwrap();
        assert_eq!(encoded.len(), 32);

        let decoded = AcceptorRemove::mls_decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(ext, decoded);
        assert_eq!(decoded.acceptor_id(), id);
    }

    #[test]
    fn test_extension_types_are_different() {
        assert_ne!(
            AcceptorAdd::extension_type(),
            AcceptorRemove::extension_type()
        );
        assert_ne!(
            AcceptorsExt::extension_type(),
            AcceptorAdd::extension_type()
        );
    }

    #[test]
    fn test_extension_types_in_private_range() {
        assert!(AcceptorsExt::extension_type().raw_value() >= 0xF000);
        assert!(AcceptorAdd::extension_type().raw_value() >= 0xF000);
        assert!(AcceptorRemove::extension_type().raw_value() >= 0xF000);
    }
}
