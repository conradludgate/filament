# Removed Member Edits — Silent Data Loss

## Problem

When a member is removed from the group via Paxos commit, their CRDT updates
that were sent before removal are silently dropped by other devices.

### Current Behaviour

In `group_actor.rs:1440-1456`, `handle_decrypted_message()` looks up the
sender in the **current** MLS roster. If the sender is not found (because
they've been removed), the message is dropped with a warning log:

```
"sender not found in roster, dropping message"
```

### Scenario

1. Device A sends CRDT updates at epoch 4 (encrypted with epoch 4 keys).
2. Device B proposes removing Device A. This wins Paxos consensus.
3. The group advances to epoch 5. Device A is no longer in the roster.
4. Device C comes online, backfills from acceptors, receives Device A's
   epoch 4 messages.
5. Device C checks the roster (now epoch 5) — Device A is not found.
6. Device A's legitimate pre-removal edits are silently dropped.

### Impact

- Data loss: edits made by a member before removal are discarded.
- The loss is silent — only a debug-level log is emitted.
- This affects any device that processes the removed member's messages
  after the removal commit.

## Design Decision Required

This behaviour could be intentional or a bug. Two valid interpretations:

### Interpretation 1: Bug — Pre-removal edits should be honoured

Most collaborative editing systems honour edits made before a member was
removed. The member had legitimate access at the time they made the edit.

Fix: look up the sender in the roster **at the epoch the message was
encrypted** (available via `AuthData`), not the current roster. If the
sender was a valid member at that epoch, process the message.

This requires retaining historical roster information or querying MLS for
past-epoch membership.

### Interpretation 2: Intentional — Removal reverts pending edits

In some security models, removing a member should also revoke their
unprocessed contributions. This is a stronger security posture but causes
data loss.

If intentional, this should be documented explicitly and the log level
should be raised to `info` or `warn` to make it visible.

## Recommendation

Honour pre-removal edits. The member had valid group access when they made
the edit. The message is cryptographically valid (signed by the member,
encrypted with valid epoch keys). Dropping it is unexpected behaviour for
a collaborative editing system.

The fix is to check membership at the message's encryption epoch rather
than the current epoch. `AuthData` already contains epoch information
that could be used for this lookup.
