# Acceptor Promise Race (TOCTOU)

**Status**: Implemented

## Problem

`AcceptorHandler::handle_prepare` and `handle_accept` performed a write
(`promise()`/`accept()`) followed by a separate read (`get()`) to build
the response. A concurrent proposer could overwrite the single-slot
promise sentinel between the write and the read, causing the read to
return `promised: None` and panicking in `from_round_state`.

### Root Cause

The spool's `GroupStateStore` stores the promised proposal at a single
sentinel key per group (not per epoch). `get_promised_sync` reads this
sentinel and filters by epoch:

```rust
fn get_promised_sync(&self, group_id: &GroupId, epoch: Epoch) -> Option<GroupProposal> {
    // reads single sentinel key, filters by epoch
    .filter(|p| p.epoch == epoch)
}
```

When proposer A promises for epoch N, then proposer B promises for
epoch N+1 before A's `get()` runs, the sentinel contains epoch N+1 and
the filter returns `None`.

### Previous Behaviour

```rust
// handler.rs — both success and rejection paths
match self.state.promise(proposal).await {
    Ok(()) => {
        let state = self.state.get(proposal.round()).await;  // ← racy read
        Ok(PromiseOutcome::Promised(AcceptorMessage::from_round_state(state)))
    }
    Err(round_state) => {
        Ok(PromiseOutcome::Outdated(AcceptorMessage::from_round_state(round_state)))
        // round_state.promised can also be None
    }
}
```

`from_round_state` called `.expect()` on `promised`, panicking.

### When This Triggers

- Multiple members driving compaction simultaneously (each advances the
  epoch via Paxos proposals)
- Rapid membership changes
- Any scenario with concurrent Paxos proposals for different epochs on
  the same group

## Current Design

Location: `filament-warp/src/acceptor/handler.rs`

All four response paths (prepare success/reject, accept success/reject)
now construct `AcceptorMessage` directly without a separate `get()`:

```rust
// Success path — we know the promised proposal
Ok(()) => {
    let accepted = self.state.get(proposal.round()).await.accepted;
    Ok(PromiseOutcome::Promised(AcceptorMessage {
        promised: proposal.clone(),
        accepted,
    }))
}

// Rejection path — fall back to the incoming proposal if promised is None
Err(round_state) => {
    Ok(PromiseOutcome::Outdated(AcceptorMessage {
        promised: round_state.promised.unwrap_or_else(|| proposal.clone()),
        accepted: round_state.accepted,
    }))
}
```

- Success paths use the proposal we just wrote — no racy sentinel read
- Rejection paths fall back to the incoming proposal when `promised` is
  `None` (the sentinel was overwritten by another epoch)
- `accepted` is still read via `get()` since it's stored per-epoch (not
  a single sentinel) and is not subject to the same race
- `from_round_state` is now `#[cfg(test)]` only
