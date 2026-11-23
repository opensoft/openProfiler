# Codex Identity and History Profile Architecture

Status: proposed  
Target: openProfiler after `0.1.4`  
Scope: local Codex profiles and local Codex chat history

## Purpose

openProfiler currently discovers provider identities and activates their local
credentials. The next capability should let a user:

1. browse Codex identity profiles independently from Codex history profiles;
2. inspect the chats stored in each history profile;
3. copy or move a chat between history profiles;
4. resume a selected chat with a selected identity;
5. hand a selected identity and chat to the Windows ChatGPT/Codex app when the
   desktop app can safely consume them.

Identity and history are separate product concepts, but Codex currently stores
authentication, sessions, configuration, and runtime state beneath one
`CODEX_HOME`. openProfiler must therefore materialize an identity/history
selection into an isolated runtime binding. It must not imply that Codex
natively supports independently selectable credential and history roots.

## Architecture Decisions

### Use Codex app-server as the integration boundary

openProfiler must use the documented Codex app-server JSON-RPC API for account
state and chat history:

- `account/read`, `account/login/start`, and `account/logout`;
- `thread/list`, `thread/read`, `thread/resume`, and `thread/fork`;
- `thread/archive` and `thread/unarchive`.

The app-server is the supported interface for rich clients that need
authentication, conversation history, approvals, and streamed agent events.
openProfiler should generate TypeScript or JSON schemas from the installed
Codex version during development and pin generated clients to that version:

```bash
codex app-server generate-ts --out ./schemas
codex app-server generate-json-schema --out ./schemas
```

SQLite is an implementation detail. openProfiler must not query, migrate, copy,
or update Codex SQLite tables. The schema is not a stable public contract.

### Treat rollout JSONL as an opaque transfer artifact

Codex persists an active thread as an append-only JSONL file beneath:

```text
$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<thread-id>.jsonl
```

Archived threads are stored beneath `$CODEX_HOME/archived_sessions`. Codex
app-server can scan these files and repair its state database when
`thread/list` is called without `useStateDbOnly`.

There is no documented Codex-to-Codex thread export/import API. Until one
exists, openProfiler may copy a rollout file between history profiles only as
an opaque file:

- never rewrite events or identifiers;
- calculate and retain a SHA-256 before and after transfer;
- validate the destination through `thread/read`;
- keep the source until destination validation succeeds;
- avoid transferring a thread while either side can write to it.

`history.jsonl` is not a substitute for a rollout. It is lightweight prompt
history and is insufficient for a complete resumable thread.

### Separate logical profiles and physical runtime bindings

The user selects an `IdentityProfile` and a `HistoryProfile`. openProfiler
creates or activates a `RuntimeBinding` that presents the selected values to
one Codex app-server process.

```text
IdentityProfile ─┐
                 ├─> RuntimeBinding -> Codex app-server
HistoryProfile ──┘          |
        |                    └─> selected credential
        └──────────────────────> physical CODEX_HOME
```

A runtime binding has one active identity and one writable history. The same
history profile must not have multiple writable bindings at the same time.

Codex does not expose an independent sessions-root setting. The history profile
therefore owns the binding's physical `CODEX_HOME`, including `sessions` and
`archived_sessions`. openProfiler atomically materializes the selected
identity's credential into that home before app-server starts and writes a
refreshed credential back to the identity profile when it stops. The runtime
credential is an active credential copy, not a second credential registry.

### Distinguish managed and external histories

History discovered in an existing Codex home is initially external and
read-only. openProfiler does not control other CLI or desktop processes that
may write there, so its own lock cannot provide exclusive access.

A history becomes writable when either:

- openProfiler created and owns its managed history home; or
- the user explicitly adopts an existing history while all other Codex writers
  are stopped.

Adoption writes non-secret ownership metadata and enables openProfiler's
single-writer lease. Commands launched outside openProfiler do not honor that
lease, so mutating operations still require a best-effort process/activity
check. The Windows app home always uses the separate desktop handoff workflow.

### Keep managed ChatGPT login as the default

The first implementation should continue using complete file-based ChatGPT
OAuth credentials and Codex-managed refresh. Before a runtime binding stops,
openProfiler saves a refreshed credential back to the matching identity
profile using the same identity verification and atomic replacement rules used
by the existing Windows desktop switch.

Codex app-server also supports experimental `chatgptAuthTokens`, where a host
application supplies access tokens and answers refresh requests. This would
decouple authentication from `CODEX_HOME`, but it makes openProfiler responsible
for the OAuth lifecycle and token refresh timing. It must remain disabled
behind an explicit experimental feature until the managed runtime design is
proven.

Credential checkout, refresh ownership, Windows/WSL synchronization, conflict
handling, and SOPS recovery policy are defined separately in
[Codex credential rotation and synchronization architecture](codex-token-management-architecture.md).

### Do not hot-switch the running Windows app

The Windows ChatGPT/Codex app uses its Windows Codex home and exposes no
documented API for hot-switching to an arbitrary `CODEX_HOME`. openProfiler
must not overwrite its session files or credentials while the app is running
as a non-owner. Reading and publishing a stable credential snapshot from the
running Windows owner is allowed; transferring refresh ownership requires
quiescing the Windows app.

The initial desktop integration is a controlled handoff of one identity and
one thread into the fixed desktop home. It is not a native desktop history
profile switch.

## Domain Model

### IdentityProfile

An existing provider login that can be activated.

```text
id
provider
name
email
family
aliases
profile_path
credential_kind
credential_present
credential_store
account_id
workspace_id
plan_type
source
```

Credential paths, tokens, and refresh material remain backend-only. The
webview receives non-secret readiness and account metadata.

### HistoryProfile

A named collection of Codex sessions.

```text
id
provider
name
family
codex_home
sessions_root
archived_sessions_root
sqlite_home
canonical_root
source
ownership
access_mode
readiness
thread_count
last_activity_at
```

History profiles can be discovered from:

1. explicit openProfiler history metadata;
2. canonicalized `sessions` paths referenced by existing Codex profiles;
3. compatible workBenches family state such as
   `~/.chatgpt-profiles/state/opensoft/sessions`;
4. the active Windows or Unix Codex home.

Profiles that resolve to the same canonical sessions root are one history
profile even when several identity profiles reference it.

`ownership` is `managed` or `external`. `access_mode` is `read_only`,
`writable`, or `busy`. Discovery never silently promotes an external history to
writable.

For compatible workBenches family state, the family state directory can become
the history profile's `codex_home`. For example,
`~/.chatgpt-profiles/state/opensoft` owns its `sessions` and
`archived_sessions` directories. A sessions-only source that cannot safely act
as a Codex home must be imported into a managed history home before it becomes
writable; openProfiler must not construct a writable runtime from undocumented
session-path overrides.

### RuntimeBinding

The active pairing used by an app-server process.

```text
id
identity_profile_id
history_profile_id
codex_home
sqlite_home
codex_version
app_server_pid
status
lease
started_at
last_verified_at
```

Statuses should include `inactive`, `starting`, `ready`, `busy`, `stopping`,
`failed`, and `recovery_required`.

### ThreadRecord

A UI projection returned by app-server, not a parsed SQLite row.

```text
thread_id
session_id
history_profile_id
name
preview
cwd
model_provider
created_at
updated_at
status
archived
forked_from_id
rollout_path
rollout_sha256
```

The rollout path and hash remain backend metadata. Chat bodies are loaded on
demand with `thread/read(includeTurns: true)`.

### TransferOperation

A journaled copy or move between history profiles.

```text
id
mode
thread_id
source_history_profile_id
destination_history_profile_id
source_path
destination_path
source_sha256
destination_sha256
state
started_at
completed_at
error
```

States should include `planned`, `source_locked`, `copied`, `validated`,
`committed`, `rolled_back`, and `failed`.

### DesktopHandoff

The recoverable state for a thread checked out to the Windows app.

```text
id
identity_profile_id
history_profile_id
thread_id
source_sha256
desktop_sha256
credential_rollback_available
state
started_at
returned_at
```

## Storage Layout

openProfiler should add a non-secret registry and runtime area without creating
a second credential database:

```text
~/.config/openprofiler/
  history-profiles.json

~/.local/share/openprofiler/
  histories/
    <history-id>/
      codex-home/
        sessions/
        archived_sessions/

~/.local/state/openprofiler/
  runtimes/
    <binding-id>/
      binding.json
  histories/
    <history-id>/
      sqlite/
  transfers/
    <operation-id>.json
  locks/
    history-<history-id>.lock
  desktop-handoffs/
    <handoff-id>.json
```

On Windows, use the corresponding per-user configuration and local-state
directories. Credential material is activated from the existing provider
profile store and is never written into `history-profiles.json`, transfer
journals, logs, or frontend state.

An explicit history registry entry should contain only a stable ID, display
metadata, and a path relative to an approved profile root where possible.
Absolute external paths require explicit user approval and the same
canonicalization checks used for identity profile discovery.

At runtime, the process environment is assembled as:

```text
CODEX_HOME=<history-profile.codex-home>
CODEX_SQLITE_HOME=<history-profile.sqlite-home>
<CODEX_HOME>/auth.json=<activated identity credential>
```

Existing compatible history homes remain in place. The managed
`~/.local/share/openprofiler/histories` area is for imported or newly created
history profiles, not for duplicating every discovered history.

Managed history homes contain no durable identity registry. Their activated
`auth.json` is removed after refreshed credentials are written back on a clean
shutdown. Recovery treats a credential left by an interrupted runtime as
sensitive pending state: verify its non-secret identity, write it back to the
matching identity profile, then remove it before activating another identity.

## Components

### Rust core

The current `open-profiler-core` crate should grow provider-neutral domain
types with Codex-specific adapters:

```text
identity/
  inventory
  activation

history/
  inventory
  registry
  transfer
  locking

codex/
  app_server
  generated_protocol
  runtime

desktop/
  codex_handoff
```

The core owns path validation, process-independent locks, transfer journals,
atomic writes, hashes, identity checks, and recovery. It must remain usable
without Tauri.

### Tauri backend

Add narrow commands rather than exposing generic filesystem or process access:

```text
history_profiles
history_threads
history_thread
copy_history_thread
move_history_thread
start_codex_runtime
stop_codex_runtime
resume_codex_thread
fork_codex_thread
desktop_handoff_start
desktop_handoff_return
desktop_handoff_rollback
```

Long operations emit typed progress events. Commands return sanitized domain
objects and structured error codes.

### React frontend

Add three work-focused views:

1. **Profiles**: existing identity inventory plus associated history profile.
2. **History**: history-profile selector, searchable thread list, thread
   details, copy, move, archive, and resume actions.
3. **Runtime**: selected identity, selected history, verified account,
   app-server state, active thread, and handoff state.

The identity selector and history selector remain visibly independent. Before
starting a runtime, show the resulting binding as:

```text
Use <identity> with <history>
```

Chat contents should not be loaded for list rows. Load them only when a user
opens thread details.

## Core Workflows

### Inventory histories

1. Discover Codex identity profiles using the existing inventory.
2. Resolve each profile's effective sessions and archived-sessions paths.
3. Canonicalize and group matching roots.
4. Merge explicit history metadata.
5. Mark externally owned roots read-only unless they have been explicitly
   adopted.
6. Start a read-only app-server for the selected history runtime.
7. Call `thread/list` with `useStateDbOnly` omitted.
8. Return sanitized thread summaries to the frontend.

Inventory failures for one history profile must not hide other valid profiles.

### Resume with a selected identity

1. Acquire the history profile's single-writer lease.
2. Create or recover the runtime binding.
3. Resolve the history profile's approved Codex home and SQLite state root.
4. Atomically activate the selected complete credential into that Codex home.
5. Start `codex app-server` with history-specific `CODEX_HOME` and
   `CODEX_SQLITE_HOME`.
6. Call `account/read` and verify the expected non-secret account identity.
7. Call `thread/read` for the selected thread.
8. Call `thread/resume`.
9. Stream activity through the app-server protocol.
10. On shutdown, wait for idle state, stop app-server, persist refreshed
    credentials to the matching identity profile, remove the runtime
    credential from managed history homes, and release the lease.

Failure to verify the account stops the runtime before any model request.

### Copy a thread

1. Confirm source and destination differ.
2. Acquire deterministic source and destination locks.
3. Verify the source thread is not active.
4. Resolve the rollout path from trusted backend inventory.
5. Hash the source and create a transfer journal.
6. Copy to a mode-restricted temporary destination.
7. Flush, hash, and atomically rename the destination.
8. Start or refresh destination app-server inventory.
9. Validate the thread with `thread/read`.
10. Commit the journal and release locks.

The source remains unchanged.

### Move a thread

Run the copy workflow first. After destination validation, archive the source by
default. Permanent deletion is a separate destructive action because Codex
deletion can also affect descendant sessions.

When a thread has known descendants, the UI must offer:

- copy only this thread;
- copy the thread tree;
- cancel.

The first implementation may reject tree moves until parent/descendant
behavior is covered by integration tests.

### Windows desktop handoff

This workflow is experimental and Windows-only:

1. Verify the selected identity has a complete reusable file credential.
2. Verify no previous handoff or credential rollback is unresolved.
3. Ask the Windows app to close and wait for all owned processes to stop.
4. Save refreshed outgoing credentials to matching identity profiles.
5. Activate the selected credential using the existing rollback mechanism.
6. Copy the selected rollout into the desktop sessions directory and journal
   its source hash.
7. Relaunch the Windows app.
8. Instruct the user to verify the account/workspace and open the transferred
   thread from chronological history.

Returning the thread:

1. Stop the Windows app.
2. Compare source, checkout, and current desktop hashes.
3. Copy changes back only when the source has not changed.
4. Report a conflict instead of overwriting concurrent changes.
5. Validate the returned thread in its source history profile.
6. Keep or roll back the desktop credential according to the existing
   confirmation flow.

This is a checked-out thread with conflict detection, not shared concurrent
editing. The feature must be labeled experimental until the desktop app offers
a supported profile or thread handoff API.

## Security and Reliability Requirements

- Never return credentials or raw auth files to the webview.
- Require complete reusable OAuth bundles; reject access-token-only profiles.
- Use mode `0600` on Unix and restricted inherited ACLs on Windows.
- Preserve the existing refusal to activate through unsafe symlinks.
- Canonicalize all history paths and require them to remain within approved
  roots.
- Treat chat transcripts as sensitive because prompts and tool output can
  contain secrets.
- Never log chat bodies, auth payloads, or raw app-server event streams by
  default.
- Stop writers before transferring a rollout.
- Treat externally discovered histories as read-only until explicit adoption.
- Detect likely external writers before every mutating operation; fail closed
  when exclusive access cannot be established.
- Use deterministic lock ordering to prevent deadlocks.
- Journal every mutating operation before the first filesystem change.
- Make all transfers restartable or rollback-capable after process failure.
- Reject a destination collision when the same thread ID has different
  content.
- Record the Codex version used for each runtime and generated protocol schema.
- Refuse unsupported protocol versions instead of falling back to SQLite.

## Delivery Plan

### Phase 1: Read-only history inventory

- Add history profile discovery and canonical-root grouping.
- Add an app-server process adapter over stdio.
- Generate and check in version-matched protocol types.
- Implement `thread/list` and `thread/read`.
- Add the History view with no mutation controls.

Exit criteria: openProfiler can display the same thread inventory and details
that Codex app-server reports for each discovered history profile without
reading SQLite directly.

### Phase 2: Runtime binding and resume

- Add single-writer leases and runtime lifecycle management.
- Activate a selected identity into a selected history runtime.
- Verify identity through `account/read`.
- Implement resume, fork, streamed events, and credential refresh write-back.

Exit criteria: a user can independently select an identity and history, resume
a thread, stop the runtime, and retain both the updated transcript and refreshed
credential in their owning profiles.

### Phase 3: Copy and move

- Add transfer journals, hashing, atomic copy, and destination validation.
- Implement copy first.
- Implement move as validated copy plus source archive.
- Add collision, interruption, and recovery tests.

Exit criteria: an interrupted operation cannot silently lose or overwrite a
thread, and every completed destination passes `thread/read`.

### Phase 4: Windows desktop handoff

- Extend the existing desktop rollback transaction with a thread checkout.
- Add source hash and conflict detection.
- Add return, keep, and rollback operations.

Exit criteria: the Windows app can resume a checked-out thread under the
selected verified identity, and openProfiler can safely return or roll back the
handoff after the app is closed.

### Phase 5: Experimental external authentication

- Evaluate `chatgptAuthTokens` behind an opt-in feature.
- Integrate a secure token provider and refresh callback.
- Threat-model token rotation, revocation, timeout, and multi-account
  concurrency.

Exit criteria: external authentication is never required for the managed
profile path and can be removed without changing history storage.

## Test Strategy

Unit tests:

- canonical root grouping and path escape rejection;
- identity/history binding validation;
- collision and duplicate-content handling;
- transfer state-machine transitions;
- deterministic lock ordering;
- credential and transcript redaction;
- desktop three-way hash comparison.

Integration tests with a temporary `CODEX_HOME`:

- generated client initializes against the supported Codex version;
- `thread/list` repairs metadata from copied JSONL;
- `thread/read`, resume, fork, archive, and unarchive;
- process interruption at every transfer state;
- app-server restart and stale-lock recovery;
- refreshed credential write-back without exposing token material.

Windows tests:

- packaged app process detection and graceful shutdown;
- ACL preservation;
- credential plus thread rollback;
- WSL profile source to Windows desktop destination;
- conflict when source and desktop copies both change.

## Non-Goals

- Editing Codex SQLite databases.
- Rewriting rollout JSONL events.
- Merging two divergent copies of one thread.
- Switching browser cookies or ChatGPT web sessions.
- Hot-switching the running Windows ChatGPT/Codex app.
- Treating Codex `--profile` as credential isolation.
- Making experimental external-token authentication the default.
- Providing Claude chat-history management through Codex app-server.

## References

- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Codex authentication](https://learn.chatgpt.com/docs/auth)
- [Codex configuration and state locations](https://learn.chatgpt.com/docs/config-file/config-advanced#config-and-state-locations)
- [Windows app: share config, auth, and sessions with WSL](https://learn.chatgpt.com/docs/windows/windows-app#share-config-auth-and-sessions-with-wsl)
- [Open-source Codex app-server](https://github.com/openai/codex/tree/main/codex-rs/app-server)
