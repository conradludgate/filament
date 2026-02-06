# Sync Editor — Implementation Plan

## Architecture

```
Frontend (JS)
    │
    ▼
Tauri Commands (thin async fns)
    │
    │  AppState holds only: mpsc::Sender<CoordinatorRequest>
    ▼
CoordinatorActor  (tokio::spawn, single instance)
    │
    ├── Owns: GroupClient (MLS + iroh + CRDT factories)
    ├── Owns: HashMap<GroupId, mpsc::Sender<DocRequest>>
    │
    ├── Handles: create_document, get_key_package, recv_welcome, join_document_bytes
    │
    └── Routes: document-specific requests → correct DocumentActor
                                              │
        ┌─────────────────────────────────────┘
        ▼
DocumentActor[group_id]  (one per open document, tokio::spawn)
    │
    ├── Owns: Group<C, CS>  (which owns the CRDT + actor handle)
    ├── Owns: tauri::AppHandle  (for emitting events)
    │
    ├── select! loop:
    │     ├── DocRequest from channel → apply_delta, get_text, add_member, etc.
    │     └── group.wait_for_update() → emit "document-updated" Tauri event
    │
    └── Handles: apply_delta, get_document_text, add_member,
                 add_acceptor, list_acceptors, shutdown
```

No mutexes anywhere. All shared state is mediated by mpsc + oneshot channels.

## Tasks

### 1. Project scaffolding

- [ ] Add `sync-editor/src-tauri` to workspace members in root `Cargo.toml`
- [ ] Create `sync-editor/src-tauri/Cargo.toml` with dependencies:
      tauri (v2), tokio, serde/serde_json,
      universal-sync-proposer, universal-sync-core, universal-sync-testing (for YrsCrdtFactory),
      yrs, iroh, mls-rs, mls-rs-crypto-rustcrypto, bs58, tracing, tracing-subscriber
- [ ] Create `sync-editor/src-tauri/build.rs` (tauri codegen build script)
- [ ] Update `sync-editor/tauri.conf.json` if needed (beforeBuildCommand, etc.)

### 2. Request / response types (`sync-editor/src-tauri/src/types.rs`)

- [ ] Define `Delta` enum (serde-tagged: `Insert { position, text }`, `Delete { position, length }`)
- [ ] Define `DocumentInfo` struct (`group_id: String`, `text: String`, `member_count: usize`)
- [ ] Define `CoordinatorRequest` enum:
      - `CreateDocument { reply }`
      - `GetKeyPackage { reply }`
      - `RecvWelcome { reply }`
      - `JoinDocumentBytes { welcome, reply }`
      - `ForDoc { group_id, request: DocRequest }`
- [ ] Define `DocRequest` enum:
      - `ApplyDelta { delta, reply }`
      - `GetText { reply }`
      - `AddMember { key_package_b58, reply }`
      - `AddAcceptor { addr_b58, reply }`
      - `ListAcceptors { reply }`
      - `Shutdown`

### 3. CoordinatorActor (`sync-editor/src-tauri/src/actor.rs`)

- [ ] Create `CoordinatorActor` struct:
      - `group_client: GroupClient<C, CS>`
      - `doc_actors: HashMap<GroupId, mpsc::Sender<DocRequest>>`
      - `request_rx: mpsc::Receiver<CoordinatorRequest>`
      - `app_handle: tauri::AppHandle`
- [ ] Implement `run()` loop: recv requests, match & dispatch
- [ ] Implement `create_document()`:
      - Call `group_client.create_group(&[], "yrs")`
      - Spawn a `DocumentActor` for the new group
      - Store its `mpsc::Sender<DocRequest>` in the routing table
      - Return `DocumentInfo`
- [ ] Implement `get_key_package()`:
      - Call `group_client.generate_key_package()`
      - Serialize to base58
- [ ] Implement `recv_welcome()`:
      - Call `group_client.recv_welcome().await`
      - Call `group_client.join_group(&welcome_bytes)`
      - Spawn a `DocumentActor` for the joined group
      - Return `DocumentInfo`
- [ ] Implement `join_document_bytes(welcome)`:
      - Call `group_client.join_group(&welcome)`
      - Spawn a `DocumentActor`
      - Return `DocumentInfo`
- [ ] Implement `ForDoc` routing:
      - Look up `doc_actors[group_id]` and forward the `DocRequest`

### 4. DocumentActor (`sync-editor/src-tauri/src/document.rs`)

- [ ] Create `DocumentActor` struct:
      - `group: Group<C, CS>`
      - `group_id_b58: String`
      - `request_rx: mpsc::Receiver<DocRequest>`
      - `app_handle: tauri::AppHandle`
- [ ] Implement `run()` select loop:
      - Branch 1: `request_rx.recv()` → handle DocRequest
      - Branch 2: `group.wait_for_update()` → read text, emit `document-updated` event
- [ ] Implement `apply_delta(delta)`:
      - Downcast `group.crdt_mut()` to `YrsCrdt`
      - Get `doc.get_or_insert_text("doc")`
      - Match delta: Insert → `text.insert()`, Delete → `text.remove_range()`
      - Call `group.send_update().await`
- [ ] Implement `get_text()`:
      - Downcast `group.crdt()` to `YrsCrdt`
      - Read text from `doc.get_or_insert_text("doc")` via `get_string()`
- [ ] Implement `add_member(key_package_b58)`:
      - Decode base58 → `MlsMessage::from_bytes`
      - Call `group.add_member(kp).await`
- [ ] Implement `add_acceptor(addr_b58)`:
      - Decode base58 → `EndpointAddr`
      - Call `group.add_acceptor(addr).await`
- [ ] Implement `list_acceptors()`:
      - Call `group.context().await`
      - Return acceptor IDs as base58 strings

### 5. Tauri commands (`sync-editor/src-tauri/src/commands.rs`)

Each command is a thin async fn that sends a request to the coordinator and awaits the oneshot reply.

- [ ] `create_document(state) → DocumentInfo`
- [ ] `get_key_package(state) → String`
- [ ] `recv_welcome(state) → DocumentInfo`
- [ ] `join_document_bytes(state, welcome) → DocumentInfo`
- [ ] `apply_delta(state, group_id, delta) → ()`
- [ ] `get_document_text(state, group_id) → String`
- [ ] `add_member(state, group_id, key_package_b58) → ()`
- [ ] `add_acceptor(state, group_id, addr_b58) → ()`
- [ ] `list_acceptors(state, group_id) → Vec<String>`

### 6. App entry point (`sync-editor/src-tauri/src/main.rs`)

- [ ] Set up tracing
- [ ] Generate or load iroh secret key
- [ ] Create MLS client with all extension types registered
- [ ] Create iroh endpoint
- [ ] Create `GroupClient`, register `YrsCrdtFactory`
- [ ] Create `mpsc::channel` for `CoordinatorRequest`
- [ ] Spawn `CoordinatorActor`
- [ ] Build Tauri app:
      - Manage `AppState { coordinator_tx }`
      - Register all `#[tauri::command]` handlers
      - Run

### 7. Frontend adjustments (`sync-editor/ui/app.js`)

- [ ] Verify `computeDelta` handles both insert and delete correctly
      (currently only returns one — acceptable for MVP, note for future improvement)
- [ ] Verify `document-updated` event handler matches `group_id` (already does)
- [ ] No other changes expected — existing JS already calls the right command names

### 8. Testing

Tests are split into three layers: unit tests for pure logic, actor tests for
message-passing correctness, and end-to-end integration tests.

#### 8a. Unit tests (no Tauri, no network)

These test pure functions and type conversions in isolation.

- [ ] `Delta` serde: round-trip `Insert` and `Delete` variants through JSON
- [ ] `Delta` application to a standalone `YrsCrdt`:
      - Insert at beginning / middle / end
      - Delete from beginning / middle / end
      - Insert then delete (replace semantics)
      - Empty insert (no-op)
      - Delete with length 0 (no-op)
      - Out-of-bounds position clamps or errors gracefully
- [ ] `DocumentInfo` serialization matches what the frontend expects
- [ ] Base58 parsing helpers: valid input, invalid input, empty input

#### 8b. Actor tests (no Tauri, real network via `sync-testing` helpers)

These spawn actors directly (without Tauri) using the same test infrastructure
as `sync-testing/tests/integration.rs`. They verify the actor message-passing
and routing logic. Use a mock or no-op `AppHandle` substitute for event emission
(e.g. capture events into an `mpsc` channel instead).

- [ ] **DocumentActor — local edit round-trip**:
      Send `ApplyDelta(Insert)` → `GetText` → verify text matches
- [ ] **DocumentActor — multiple sequential edits**:
      Send several `ApplyDelta`s → `GetText` → verify accumulated text
- [ ] **DocumentActor — delete after insert**:
      Insert text → delete part of it → `GetText` → verify
- [ ] **DocumentActor — remote update triggers event**:
      Two `Group`s in the same MLS group (Alice & Bob).
      Alice's DocumentActor applies a delta and sends an update.
      Bob's DocumentActor receives the update via `wait_for_update()`
      and emits a `document-updated` event (captured via channel).
      Verify Bob's text matches Alice's.
- [ ] **CoordinatorActor — create document**:
      Send `CreateDocument` → verify `DocumentInfo` returned with valid group_id
      Send `ForDoc { GetText }` → verify empty text for new document
- [ ] **CoordinatorActor — create multiple documents**:
      Create two documents → verify different group_ids
      Apply deltas to each independently → verify texts are independent
- [ ] **CoordinatorActor — route to correct DocumentActor**:
      Create two documents. Apply delta to doc A. GetText from doc A (has text)
      and doc B (still empty) — verify isolation.
- [ ] **CoordinatorActor — route to unknown group_id**:
      Send `ForDoc` with a non-existent group_id → verify the oneshot reply
      is dropped (caller receives an error, no panic)
- [ ] **CoordinatorActor — join via welcome**:
      Use two CoordinatorActors (Alice and Bob, each with their own GroupClient).
      Alice creates a document and adds Bob. Bob receives the welcome, joins,
      and verifies the document text matches.
- [ ] **Full two-peer sync**:
      Alice creates doc, adds acceptor, adds Bob.
      Alice types "Hello" → Bob sees "Hello" (via event).
      Bob types " World" → Alice sees "Hello World" (via event).

#### 8c. Frontend / E2E tests (optional, manual for now)

These require the full Tauri app running. Document as manual test scripts
rather than automated tests for now.

- [ ] Launch app → "No document open" empty state shown
- [ ] Click "Create New Document" → editor appears, sync status shows "Synced"
- [ ] Type text → no errors, sync indicator stays green
- [ ] Click "Join" → invite code generated, modal shows "Waiting to be added..."
- [ ] Two-window test: Window A creates doc + adds acceptor.
      Window B clicks Join, copies invite code. Window A pastes invite code
      in Add Member. Window B auto-joins. Typing in A appears in B and vice versa.
- [ ] Acceptor modal: add/list acceptors works correctly
