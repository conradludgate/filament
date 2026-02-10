# External Commit Join

**Status**: Implemented

## Problem

The external commit join flow required the inviter to encrypt the
`GroupInfo` with AES-256-GCM, store the ciphertext on a spool, and share
decryption material (key, nonce, spool ID, group ID) via a QR code. The
joiner then fetched the ciphertext from the spool and decrypted it before
performing the MLS external commit.

### Previous Behaviour

- `QrPayload { spool_id, group_id, key: [u8; 32], nonce: [u8; 12] }` (~108 bytes)
- `Handshake::StoreGroupInfo` / `Handshake::FetchGroupInfo` protocol messages
- `encrypt_group_info` / `decrypt_group_info` helpers using `aes-gcm`
- GroupInfo included the ratchet tree inline (`with_tree_in_extension: true`)

## Current Design

The QR code payload is now the raw MLS `GroupInfo` bytes (without the
ratchet tree). The joiner fetches only the ratchet tree from a spool.

### Flow

1. Inviter generates `GroupInfo` **without** the ratchet tree
   (`with_tree_in_extension: false`). The `GroupInfoExt` extension
   carries the acceptor list and inviter fingerprint.
2. Raw `GroupInfo` bytes (~330-370 bytes) are base58-encoded into a QR code.
3. Joiner decodes the QR, parses the `GroupInfo` to extract `group_id`,
   `acceptor_ids` (from `GroupInfoExt`), and `confirmed_transcript_hash`.
4. Joiner connects to any acceptor, sends `Handshake::FetchTree { group_id, confirmed_transcript_hash }`.
5. Spool verifies the transcript hash matches its `ExternalGroup` state,
   then calls `ExternalGroup::export_tree()` and returns the bytes.
6. Joiner calls `ExternalCommitBuilder::with_tree_data(tree).build(group_info_msg)`.
7. External commit is submitted to the spool.

### Key types and modules

- `Weaver::generate_external_group_info()` — generates the GroupInfo bytes
- `parse_group_info_metadata()` in `filament-weave/src/group/mod.rs` —
  extracts `GroupInfoMetadata { group_id, acceptor_ids, confirmed_transcript_hash }`
- `Handshake::FetchTree` in `filament-core/src/protocol.rs`
- `GroupAcceptor::export_tree()` / `AcceptorRegistry::export_tree()` in `filament-spool`
- `WeaverClient::join_external(&self, group_info_bytes: &[u8])` in `filament-weave/src/client.rs`

### Removed

- `QrPayload` struct
- `encrypt_group_info` / `decrypt_group_info` functions
- `aes-gcm` dependency from `filament-core`
- `Handshake::StoreGroupInfo` / `Handshake::FetchGroupInfo`
- `encrypted_group_info` in-memory storage on the spool

## Not Yet Implemented

The spool only returns the tree if `confirmed_transcript_hash` matches the
current epoch. If the group has advanced past the QR's epoch, the join
fails. A small window of recent tree snapshots could mitigate stale QR
codes.
