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

### Consequences

- N-way Paxos contention on every compaction round
- Under high throughput (stress test with 10 members), concurrent
  compaction proposals caused epoch divergence — members advanced to
  different epochs and could no longer decrypt each other's messages
- Acceptor panics from the promise sentinel race
  (see [ACCEPTOR-PROMISE-RACE.md](ACCEPTOR-PROMISE-RACE.md))

## Current Design

Location: `filament-weave/src/group/group_actor.rs`

### Leader Election

Before emitting `CompactionNeeded`, the group actor calls
`should_drive_compaction(level)` which deterministically picks one member
per (epoch, level) pair:

```rust
fn should_drive_compaction(&self, level: u8) -> bool {
    let member_count = roster().members().len();
    let my_index = current_member_index() as usize;
    let epoch = mls_epoch().0;
    let hash = xxhash_rust::xxh3::xxh3_64(&[&epoch.to_le_bytes()[..], &[level]].concat());
    hash % (member_count as u64) == (my_index % member_count) as u64
}
```

- Uses xxh3 over `(epoch, level)` to pick a deterministic slot
- Compares against the member's own MLS roster index
- Different epochs and levels elect different leaders
- Forced compaction (member add/join) bypasses the check — always emitted

### Properties

- Expected 1 member drives each compaction round
- No coordination or extra messages required
- Deterministic: all members agree on who should compact (assuming
  consistent roster view at the same epoch)
- Degrades gracefully: if the elected member is offline, compaction is
  delayed until the next epoch/level change elects a different member

### Interaction with Claim Mechanism

The existing `has_active_claim(level)` check still runs before the leader
election check. If another member already claimed the compaction (e.g.,
from a prior epoch), the event is suppressed regardless of the election
result. The two mechanisms are complementary:

1. `has_active_claim` — prevents duplicate compaction within a claim window
2. `should_drive_compaction` — prevents N-way contention at the start

## Not Yet Implemented

- **Fallback timeout**: If the elected leader is offline and doesn't
  compact, other members don't currently step in. A timeout-based
  fallback (e.g., "if no compaction within 30s, any member may try")
  would improve resilience.
