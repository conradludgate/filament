# Key Rotation

## Problem

There is no time-based key rotation floor. Key rotation occurs only when
triggered manually or as a side effect of compaction commits. In a
low-activity group, keys may never rotate.

### Current Behaviour

- `handle_update_keys()` in `group_actor.rs:559-577` builds an empty
  commit with `UpdateKeys` to trigger key rotation. This is a manual
  operation.
- Compaction commits include `UpdateKeys` proposals, providing key
  rotation as a side effect.
- Compaction is triggered by message count thresholds
  (`CompactionConfig`), not by time.

### Consequences

MLS provides post-compromise security (PCS) through epoch advancement.
If a device's keys are compromised, the attacker can decrypt messages
until the group advances to a new epoch. Without time-based rotation:

- A group that is rarely edited could go weeks/months without rotating.
- The PCS window is unbounded for quiet groups.
- An attacker who compromises a device key can silently read all future
  messages until activity resumes and triggers compaction.

### When This Matters

- Groups with infrequent edits (e.g., a shared document edited weekly).
- After a suspected device compromise where the group is not actively
  being used.

## Proposed Fix

Add a periodic key rotation timer to `GroupActor`. When no commit has
occurred for a configurable duration (e.g., 24 hours), automatically
propose an `UpdateKeys` commit.

Implementation:
1. Track `last_epoch_advance: Instant` in `GroupActor`.
2. Add a `tokio::time::interval` that checks if the duration since
   `last_epoch_advance` exceeds the configured threshold.
3. If so, call `handle_update_keys()`.
4. Reset `last_epoch_advance` on every epoch advance.

The threshold should be configurable per group, with a sensible default
(e.g., 24 hours). For high-security groups, this could be set lower
(e.g., 1 hour).

Contention risk is low: if multiple devices independently trigger
rotation, only one wins via Paxos and the others observe the epoch
advance, resetting their timer.
