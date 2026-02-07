# Backoff and Contention

## Problem

The retry/backoff strategy for Paxos proposal contention is linear with no
jitter and a low retry limit.

### Current Behaviour

Location: `sync-proposer/src/group/group_actor.rs`

```
sleep(10ms * (retries + 1))
```

- Backoff: 10ms, 20ms, 30ms (linear)
- Max retries: 3 (`MAX_PROPOSAL_RETRIES`)
- No jitter

The README describes "exponential backoff with jitter" but the code
implements linear backoff with no jitter.

### Consequences

1. **Correlated retries**: Without jitter, two devices that collide on
   attempt 0 will retry at nearly the same time (10ms later), collide
   again, retry at 20ms, collide again, and exhaust retries.

2. **Low retry budget**: 3 retries may not be enough under contention.
   With N devices all trying to commit simultaneously (e.g., after a
   network partition heals), the probability of success within 3 attempts
   decreases with N.

3. **Fast failure**: After 3 retries (total ~60ms of backoff), the
   proposal fails with "max proposal retries exceeded". The caller must
   handle this. In practice, this surfaces as a user-visible error.

### When This Matters

- Rare in normal operation (commits are infrequent — add/remove members,
  key rotation, compaction).
- More likely after network partition recovery, where multiple devices
  may have queued commits.
- More likely with many group members who independently decide to rotate
  keys or trigger compaction simultaneously.

## Proposed Fix

Replace linear backoff with exponential backoff and jitter:

```rust
let base = 10u64;
let max_delay = 1000u64; // 1 second cap
let delay = base.saturating_mul(1u64 << retries.min(6));
let jitter = rand::random::<u64>() % (delay / 2 + 1);
sleep(Duration::from_millis(delay + jitter).min(Duration::from_millis(max_delay)))
```

Consider increasing `MAX_PROPOSAL_RETRIES` to 5-7 to handle higher
contention scenarios.

Update the attempt increment to use the superseding proposal's attempt
as the floor (this is already done — `next_attempt` uses the superseding
attempt).
