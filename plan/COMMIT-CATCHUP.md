# Commit Catch-Up Gap

## Problem

When a proposer (device) reconnects after being offline, it has no mechanism
to proactively learn about commits that occurred while it was away.

### Current Behaviour

1. On reconnect, `AcceptorActor` opens a proposal stream and a message stream.
2. The message stream requests backfill with an empty `StateVector` — so
   application messages are recovered.
3. The proposal stream does **not** request historical commits. The proposer
   only learns about past commits indirectly:
   - By attempting its own proposal and receiving `accepted` values in the
     Paxos response.
   - By failing to decrypt application messages (triggering the pending
     message buffer), which is an indirect signal.

### Consequences

- A device that restarts and has no reason to propose will **never** learn
  about commits that happened while it was offline.
- Its MLS epoch is stale (persisted to disk via `FjallGroupStateStorage`).
- Application messages encrypted with newer epoch keys will fail to decrypt.
- These are buffered in `pending_messages` but dropped after 10 attempts or
  if the device is 5+ epochs past join epoch with 3+ failed attempts.
- The device effectively becomes stuck — it can't process new messages and
  can't advance its epoch.

### Additional State Loss on Crash

The following proposer state is in-memory only (lost on crash):

- `active_proposal` — pending proposal state
- `attempt` — current proposal attempt number
- `quorum_tracker` — quorum tracking state
- `state_vector` — per-sender sequence numbers (reset to empty, causing
  full re-backfill — wasteful but correct)
- `seen_messages` — deduplication set (reset, causing duplicate processing
  — idempotent due to CRDTs)
- `message_seq` — next outgoing sequence number

The `message_seq` reset is potentially problematic: if the device sends a
message with a seq that was already used pre-crash, the acceptor would
overwrite the old message (same key in storage). This could cause data loss
if the old message hasn't been received by all peers yet.

## Proposed Solution

### Option A: Commit Backfill on Proposal Stream

Add a `SubscribeCommits { since_epoch: Epoch }` handshake/request on the
proposal stream. Acceptors already store all accepted proposals
(`get_accepted_from(epoch)` exists in `FjallStateStore`). The acceptor
sends all committed proposals from `since_epoch` onwards, then switches to
live subscription.

Advantages:
- Mirrors the existing message backfill pattern
- Acceptor already has the query (`get_accepted_from_sync`)
- Device knows its last epoch from persisted MLS state

Implementation:
1. Add `ProposalRequest::Subscribe { since_epoch: Epoch }` variant
2. On proposal stream open, send subscribe request before any proposals
3. Acceptor responds with historical accepted values
4. Proposer processes them sequentially via `apply_proposal()`

### Option B: Derive Epoch from Application Messages

When application messages fail to decrypt, use this as a signal that the
epoch is stale and trigger a "probe" proposal (e.g., an empty commit or
UpdateKeys) to force learning via Paxos responses.

Advantages:
- No protocol changes needed

Disadvantages:
- Indirect, fragile, depends on receiving undecryptable messages
- Adds unnecessary Paxos rounds
- Doesn't work if no new messages are being sent

### Option C: Persist Proposer State

Persist `message_seq`, `state_vector`, and epoch-related state to disk.
This doesn't solve the catch-up problem but reduces waste on restart.

**Recommendation**: Option A, possibly combined with Option C for
`message_seq` persistence to prevent sequence number reuse.

## `message_seq` Reuse Risk

On crash, `message_seq` resets to 0. If the device had already sent messages
with seq 0, 1, 2, ... pre-crash, the new messages would reuse those sequence
numbers. Since the acceptor key is `(group_id, sender_fingerprint, seq)`,
this overwrites the old messages.

Fix: persist `message_seq` to disk, or derive it from the acceptor during
backfill (request the highest seq for this sender).
