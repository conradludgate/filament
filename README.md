# universal-sync

Federated sync engine built on MLS, Paxos, iroh, and CRDTs.

Synchronise documents between multiple devices through a federated set of
untrusted servers, with end-to-end encryption and local-first editing.
Devices exchange encrypted CRDT updates through the servers; the shared
document is eventually consistent without the servers ever seeing its contents.

## Architecture

![](readme/sync.png)

### Overview

Devices communicate through a small set of federated **acceptor** servers.
These servers are untrusted — they cannot read or forge messages — but are
expected to maintain availability and not withhold messages. Running multiple
servers under independent administrators makes availability and withholding
attacks impractical. Servers can be added or removed at any time.

All messages are signed and encrypted with MLS, providing forward secrecy and
post-compromise secrecy. The servers see group metadata (membership, timing)
but never message contents.

There are two classes of messages with different delivery paths:

- **Application messages** (CRDT updates) are local-first. They bypass
  consensus and are routed to a subset of acceptors for storage. They can be
  sent and received asynchronously, including while offline.
- **Commits** (membership changes, key rotation, compaction) require global
  ordering. They go through Paxos consensus across all acceptors.

```mermaid
graph LR
    subgraph Devices
        D1[Device A]
        D2[Device B]
        D3[Device C]
    end

    subgraph Acceptors
        A1[Acceptor 1]
        A2[Acceptor 2]
        A3[Acceptor 3]
    end

    D1 -- "proposal stream\n(Paxos)" --- A1
    D1 -- "proposal stream" --- A2
    D1 -- "proposal stream" --- A3
    D1 -. "message stream\n(CRDT updates)" .-> A1
    D1 -. "message stream" .-> A2

    D2 -- "proposal stream" --- A1
    D2 -- "proposal stream" --- A2
    D2 -- "proposal stream" --- A3
    D2 -. "message stream" .-> A2
    D2 -. "message stream" .-> A3

    D3 -- "proposal stream" --- A1
    D3 -- "proposal stream" --- A2
    D3 -- "proposal stream" --- A3
    D3 -. "message stream" .-> A1
    D3 -. "message stream" .-> A3
```

Solid lines: proposal streams (all acceptors). Dotted lines: message streams
(subset via rendezvous hashing).

### Crate structure

| Crate | Role |
|-------|------|
| `paxos` | Core Multi-Paxos state machines (pure + async runtime) |
| `sync-core` | Shared types, protocol messages, CRDT traits, MLS extensions |
| `sync-proposer` | Device/client library — group management, message routing |
| `sync-acceptor` | Server library — Paxos acceptor, message storage, backfill |
| `sync-testing` | Integration test harness |
| `sync-editor` | Tauri desktop editor application |

## MLS ([Messaging Layer Security](https://en.wikipedia.org/wiki/Messaging_Layer_Security))

MLS provides the group key agreement protocol. Each document has its own MLS
group. MLS handles:

- Encrypting and signing all messages (application messages and commits).
- Efficiently adding and removing members from the sync group.
- Epoch-based key rotation for forward secrecy.

Custom MLS extensions carry protocol-specific data:

| Extension / Proposal | Purpose |
|-----------------------|---------|
| `GroupContextExt` | CRDT type identifier and compaction config |
| `KeyPackageExt` | Member iroh address and supported CRDT types |
| `GroupInfoExt` | Current acceptor list and optional CRDT snapshot |
| `SyncProposal::AcceptorAdd` | Add an acceptor server to the group |
| `SyncProposal::AcceptorRemove` | Remove an acceptor server from the group |
| `SyncProposal::CompactionClaim` | Claim a compaction lease |
| `SyncProposal::CompactionComplete` | Mark compaction as finished |

## Paxos

MLS application messages work asynchronously, but MLS commits must be
processed in a globally consistent order. Paxos provides this serialisation.

**Roles:**

- **Proposers** — devices proposing a commit (add/remove member, update keys,
  compaction). Each device acts as an independent proposer; there is no leader
  election.
- **Acceptors** — the federated servers. They run the Paxos acceptor role,
  storing promises and accepted values. Quorum is `floor(n/2) + 1`.
- **Learners** — all devices and acceptors. They observe accepted commits and
  advance the MLS epoch.

**Ballot scheme:** `(Epoch, Attempt, MemberId)` with lexicographic ordering.
`MemberId` is the MLS leaf index, guaranteed unique within the group.

Every round requires two phases (Prepare → Promise, Accept → Accepted).
Multi-Paxos leader optimisation is explicitly disabled — every Accept must
match an exact prior Promise. Contention between concurrent proposers is
resolved by incrementing the attempt number and retrying.

```mermaid
sequenceDiagram
    participant D as Device (Proposer)
    participant A1 as Acceptor 1
    participant A2 as Acceptor 2
    participant A3 as Acceptor 3

    Note over D: Prepare MLS commit

    D->>A1: Prepare(epoch, attempt, member_id)
    D->>A2: Prepare(epoch, attempt, member_id)
    D->>A3: Prepare(epoch, attempt, member_id)

    A1-->>D: Promise
    A2-->>D: Promise

    Note over D: Quorum (2/3) reached

    D->>A1: Accept(proposal, mls_commit)
    D->>A2: Accept(proposal, mls_commit)
    D->>A3: Accept(proposal, mls_commit)

    A1-->>D: Accepted
    A2-->>D: Accepted

    Note over D: Quorum accepted — apply commit

    A1-->>A2: Accepted (gossip)
    A1-->>A3: Accepted (gossip)
    A2-->>A1: Accepted (gossip)
    A2-->>A3: Accepted (gossip)

    Note over A1,A3: All acceptors learn and advance epoch
```

Acceptor membership changes (`AcceptorAdd` / `AcceptorRemove`) are themselves
MLS custom proposals agreed through Paxos. Quorum size updates atomically
when the commit is applied.

## iroh

All network communication uses [iroh](https://www.iroh.computer/). Devices
connect to acceptor servers over iroh, which provides:

- NAT traversal and hole punching.
- Relay-based fallback transport.
- Multiplexed bidirectional streams.

Each device opens two streams per acceptor:

1. **Proposal stream** — bidirectional Paxos Prepare/Accept messages.
2. **Message stream** — application message send, subscribe, and backfill.

```mermaid
graph TB
    subgraph Device
        GA[GroupActor]
        AA1[AcceptorActor 1]
        AA2[AcceptorActor 2]
        GA --> AA1
        GA --> AA2
    end

    subgraph "Acceptor 1"
        PS1[Proposal Stream]
        MS1[Message Stream]
    end

    subgraph "Acceptor 2"
        PS2[Proposal Stream]
        MS2[Message Stream]
    end

    AA1 -- "iroh BiDi" --> PS1
    AA1 -- "iroh BiDi" --> MS1
    AA2 -- "iroh BiDi" --> PS2
    AA2 -- "iroh BiDi" --> MS2
```

## CRDTs ([Conflict-free Replicated Data Types](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type))

Document changes are local-first. Edits happen offline and synchronise when
connectivity is restored. Since there is no global ordering of edits, CRDTs
ensure all devices converge to the same document state.

CRDT updates are MLS application messages. They are:

- Identified by `(MemberFingerprint, seq)` where `MemberFingerprint` is a
  SHA-256 of the sender's MLS signing key.
- Deduplicated on both the acceptor (storage key) and device (in-memory set).
- Tracked with a per-sender state vector for backfill.

### Message routing

Not every application message needs to reach every acceptor. Messages are
routed to `ceil(sqrt(n))` acceptors selected by rendezvous hashing
(Highest Random Weight). This distributes storage load while maintaining
deterministic routing — any device can compute which acceptors hold a given
message.

Devices connect to all acceptors and subscribe to live broadcasts plus
backfill, so they receive the full message set across all acceptors.

```mermaid
sequenceDiagram
    participant D as Device A
    participant A1 as Acceptor 1
    participant A2 as Acceptor 2
    participant A3 as Acceptor 3
    participant E as Device B

    Note over D: Edit document locally

    D->>D: Encrypt CRDT update (MLS)
    D->>A1: Send(message_id, ciphertext)
    D->>A2: Send(message_id, ciphertext)
    Note right of A3: Not selected by<br/>rendezvous hash

    A1-->>E: Broadcast(message_id, ciphertext)
    E->>E: Decrypt & apply CRDT update

    Note over E: Comes online later, backfills

    E->>A1: Backfill(state_vector)
    E->>A2: Backfill(state_vector)
    E->>A3: Backfill(state_vector)
    A1-->>E: Messages not in state_vector
    A2-->>E: Messages not in state_vector
    A3-->>E: (none stored)
    E->>E: Deduplicate & apply
```

### Compaction

Over time, CRDT updates accumulate. Hierarchical LSM-style compaction merges
older updates into compressed snapshots. A device claims a compaction lease
via Paxos (`CompactionClaim`), merges the covered updates, stores the
compacted result as a new message with `AuthData::Compaction` metadata, and
finalises via `CompactionComplete`. Acceptors then delete the superseded
messages.

Compacted snapshots serve as catch-up points for offline or new devices —
they backfill the snapshot plus any updates after the compaction watermark.

```mermaid
sequenceDiagram
    participant D as Compactor Device
    participant P as Paxos (Acceptors)
    participant S as Acceptor Storage

    D->>P: Commit(CompactionClaim{level, watermark, deadline})
    P-->>D: Accepted

    D->>S: Fetch messages covered by watermark
    S-->>D: Messages

    D->>D: Merge into compacted snapshot

    D->>S: Store snapshot (AuthData::Compaction)
    D->>P: Commit(CompactionComplete{level, watermark})
    P-->>D: Accepted

    S->>S: delete_before_watermark()
```

## Acceptor storage

Acceptors use [fjall](https://github.com/fjall-rs/fjall) (an LSM-tree
storage engine) with the following keyspaces:

| Keyspace | Key | Value |
|----------|-----|-------|
| `promised` | `(group_id, epoch)` | Promised proposal |
| `accepted` | `(group_id, epoch)` | Accepted (proposal, message) |
| `groups` | `group_id` | GroupInfo bytes |
| `messages` | `(group_id, sender_fingerprint, seq)` | Encrypted application message |
| `epoch_rosters` | `(group_id, epoch)` | Historical roster snapshot |

All writes are fsynced before acknowledging. Historical commits are replayed
from epoch 0 on server restart.

## Trust model

| Property | Guarantee |
|----------|-----------|
| Message confidentiality | MLS encryption; acceptors cannot read contents |
| Message integrity | MLS signatures + AAD; acceptors cannot forge or reorder |
| Availability | Federated; quorum of acceptors required |
| Forward secrecy | MLS epoch-based key rotation |
| Post-compromise secrecy | MLS key rotation on commits |

Acceptors are untrusted for secrecy and integrity. They are semi-trusted for
availability — a colluding quorum can cause denial of service but cannot
compromise message contents. Metadata (group membership, message timing and
frequency) is visible to acceptors.

Members within a group are fully trusted.
