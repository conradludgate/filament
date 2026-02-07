//! Postcard codec for length-delimited framing with serde serialization.
//!
//! Provides two codecs:
//! - [`PostcardCodec`]: plain postcard serialization (for unversioned messages like Handshake)
//! - [`VersionedCodec`]: version-dispatched deserialization driven by a `protocol_version`

use std::io;
use std::marker::PhantomData;

use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};

/// Version-dispatched serialization/deserialization.
///
/// Types that implement this trait can be serialized and deserialized
/// according to a protocol version read from the group's `GroupContextExt`.
pub trait Versioned: Sized {
    /// Serialize using the given protocol version's schema.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    fn serialize_versioned(&self, protocol_version: u32) -> Result<Vec<u8>, postcard::Error>;

    /// Deserialize from the given protocol version's schema.
    ///
    /// # Errors
    ///
    /// Returns an error if the version is unknown or deserialization fails.
    fn deserialize_versioned(protocol_version: u32, bytes: &[u8]) -> Result<Self, postcard::Error>;
}

fn new_length_delimited_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .max_frame_length(16 * 1024 * 1024)
        .new_codec()
}

// ---------------------------------------------------------------------------
// PostcardCodec — plain postcard (for unversioned messages)
// ---------------------------------------------------------------------------

/// Wraps [`LengthDelimitedCodec`] with automatic postcard serialization.
#[derive(Debug)]
pub struct PostcardCodec<T> {
    inner: LengthDelimitedCodec,
    _marker: PhantomData<T>,
}

impl<T> Clone for PostcardCodec<T> {
    fn clone(&self) -> Self {
        Self {
            inner: new_length_delimited_codec(),
            _marker: PhantomData,
        }
    }
}

impl<T> Default for PostcardCodec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PostcardCodec<T> {
    /// Max frame length: 16 MB.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: new_length_delimited_codec(),
            _marker: PhantomData,
        }
    }
}

impl<T> Decoder for PostcardCodec<T>
where
    T: for<'de> Deserialize<'de>,
{
    type Item = T;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.inner.decode(src)? {
            Some(bytes) => {
                let item = postcard::from_bytes(&bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some(item))
            }
            None => Ok(None),
        }
    }
}

impl<T> Encoder<T> for PostcardCodec<T>
where
    T: Serialize,
{
    type Error = io::Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = postcard::to_allocvec(&item)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.inner.encode(Bytes::from(bytes), dst)
    }
}

// ---------------------------------------------------------------------------
// VersionedCodec — version-dispatched serialization
// ---------------------------------------------------------------------------

/// Length-delimited codec that uses [`Versioned`] for version-dispatched
/// serialization/deserialization. The `protocol_version` is read from the
/// group's `GroupContextExt` and can be updated when a commit changes it.
#[derive(Debug)]
pub struct VersionedCodec<T> {
    inner: LengthDelimitedCodec,
    protocol_version: u32,
    _marker: PhantomData<T>,
}

impl<T> VersionedCodec<T> {
    #[must_use]
    pub fn new(protocol_version: u32) -> Self {
        Self {
            inner: new_length_delimited_codec(),
            protocol_version,
            _marker: PhantomData,
        }
    }

    pub fn set_protocol_version(&mut self, version: u32) {
        self.protocol_version = version;
    }

    #[must_use]
    pub fn protocol_version(&self) -> u32 {
        self.protocol_version
    }
}

impl<T> Decoder for VersionedCodec<T>
where
    T: Versioned,
{
    type Item = T;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.inner.decode(src)? {
            Some(bytes) => {
                let item = T::deserialize_versioned(self.protocol_version, &bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some(item))
            }
            None => Ok(None),
        }
    }
}

impl<T> Encoder<T> for VersionedCodec<T>
where
    T: Versioned,
{
    type Error = io::Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = item
            .serialize_versioned(self.protocol_version)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.inner.encode(Bytes::from(bytes), dst)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tokio_util::codec::{Decoder, Encoder};

    use super::*;
    use crate::protocol::{AuthData, MessageRequest, MessageResponse};

    #[test]
    fn versioned_auth_data_roundtrip() {
        let data = AuthData::update(42);
        let bytes = data.serialize_versioned(1).unwrap();
        let decoded = AuthData::deserialize_versioned(1, &bytes).unwrap();
        assert_eq!(decoded.seq(), 42);
    }

    #[test]
    fn versioned_auth_data_unknown_version() {
        let data = AuthData::update(1);
        assert!(data.serialize_versioned(99).is_err());
        assert!(AuthData::deserialize_versioned(99, &[0]).is_err());
    }

    #[test]
    fn versioned_codec_roundtrip() {
        let mut codec: VersionedCodec<MessageRequest> = VersionedCodec::new(1);
        let request = MessageRequest::Subscribe {
            state_vector: BTreeMap::default(),
        };

        let mut buf = BytesMut::new();
        codec.encode(request, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert!(matches!(decoded, MessageRequest::Subscribe { .. }));
    }

    #[test]
    fn versioned_codec_version_update() {
        let mut codec: VersionedCodec<MessageResponse> = VersionedCodec::new(1);
        assert_eq!(codec.protocol_version(), 1);

        codec.set_protocol_version(2);
        assert_eq!(codec.protocol_version(), 2);
    }
}
