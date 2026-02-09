# Compaction Leader Election

**Status**: Implemented

## Problem

When the compaction threshold is reached, every group member independently
emits `CompactionNeeded` and the application calls `compact()`. With
multiple members active, this causes concurrent Paxos proposals for the
same compaction round. Only one proposal can win; the rest contend, retry,
and waste resources.

### Previous Behaviour

`handle_send_message` checked `cascade_target()` and `has_active_claim()`
but did not consider whether *this* member should be the one to drive
compaction. Every member that saw the threshold would emit the event.

Force-compaction paths (`Add`, `ExternalInit`) emitted `CompactionNeeded`
on **all** weavers unconditionally.

An unused `CompactionClaim` scaffolding (`submit_compaction_claim`,
`ActiveCompactionClaim`, `active_claims`, `has_active_claim`,
`COMPACTION_DEADLINE_SECS`) existed but was never called.

### Consequences

- N-way Paxos contention on every compaction round
- Under high throughput (stress test with 10 members), concurrent
  compaction proposals caused epoch divergence — members advanced to
  different epochs and could no longer decrypt each other's messages
- Acceptor panics from the promise sentinel race
  (see [ACCEPTOR-PROMISE-RACE.md](ACCEPTOR-PROMISE-RACE.md))

## Current Design

Location: `filament-weave/src/group/group_actor.rs`, `filament-weave/src/group/mod.rs`, `filament-core/src/extension.rs`

### Leader Election (threshold compaction)

Before emitting `CompactionNeeded`, the group actor calls
`should_drive_compaction(level)` which deterministically picks one member
using the state vector:

```rust
fn should_drive_compaction(&self, level: u8) -> bool {
    let member_count = roster().members().len();
    let my_index = current_member_index() as usize;
    let divisor = u64::from(COMPACTION_BASE);
    let mut hasher_input = Vec::new();
    hasher_input.push(level);
    for (fp, &seq) in &self.state_vector {
        hasher_input.extend_from_slice(&fp.0);
        hasher_input.extend_from_slice(&(seq / divisor).to_le_bytes());
    }
    let hash = xxhash_rust::xxh3::xxh3_64(&hasher_input);
    hash % (member_count as u64) == (my_index % member_count) as u64
}
```

- Hashes `(level, state_vector / COMPACTION_BASE)` to pick a slot
- `BTreeMap` iteration gives sorted key order so all weavers produce the same hash
- The chosen compactor rotates roughly every `COMPACTION_BASE` messages
  from any member, with no extra bookkeeping

### Add compaction gating

`Add` proposals only emit `CompactionNeeded { force: true }` on the
committer (the weaver that issued the Add commit), checked via
`committer_fingerprint == Some(self.own_fingerprint)`.

### ExternalInit compaction (invited_by)

When a member joins via external commit, the inviter's fingerprint is
embedded in the MLS commit's `authenticated_data` so only the inviter
compacts:

1. `GroupInfoExt` carries `invited_by: Option<MemberFingerprint>`, set
   when generating GroupInfo for external commit
2. The joiner extracts it and passes it via
   `ExternalCommitBuilder::with_authenticated_data()`
3. Existing weavers read `CommitMessageDescription::authenticated_data`
   and decode the inviter fingerprint
4. Only the inviter emits `CompactionNeeded { force: true }`

If `authenticated_data` is empty or unparsable (old clients), no weaver
compacts on that `ExternalInit` — the threshold path catches up eventually.

### Cascade guard in handle_compact_snapshot

Before executing compaction, `handle_compact_snapshot` re-checks
`cascade_target()`. If another weaver's `CompactionComplete` was processed
since `CompactionNeeded` was emitted, the counts will have been reset and
the compaction is skipped. This guard is bypassed for forced compactions
(Add/ExternalInit).

### Dead code cleanup

Removed unused `CompactionClaim` scaffolding:
- `submit_compaction_claim`, `ActiveCompactionClaim`, `active_claims`,
  `has_active_claim`, `prune_expired_claims`, `COMPACTION_DEADLINE_SECS`,
  `CompactionClaimed` event variant

`SyncProposal::CompactionClaim` has been removed from `filament-core`,
`filament-weave`, and `filament-spool`.

### Properties

- Expected 1 member drives each compaction round
- No coordination or extra messages required
- Deterministic: all members agree on who should compact (assuming
  consistent state vector view)
- Rotates: the elected compactor changes as the state vector advances
- Degrades gracefully: if the elected member is offline, compaction is
  delayed until the state vector changes elect a different member

## Not Yet Implemented

- **Fallback timeout**: If the elected leader is offline and doesn't
  compact, other members don't currently step in.
