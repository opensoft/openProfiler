# Codex Credential Rotation and Synchronization Architecture

Status: proposed  
Target: openProfiler after `0.1.4`  
Scope: one workstation with Windows and WSL Codex clients

Related feature: [Cloud Credential Vault and Broker](cloud-credential-vault-feature.md)

This document defines local Codex refresh ownership and runtime
synchronization. The cloud credential feature extends the same single-writer
model across workstations. Once implemented, its exact Key Vault secret version
becomes the durable accepted credential generation; local WSL profile files
become checked-out runtime copies rather than the remote authority.

## Purpose

openProfiler manages Codex identity profiles in WSL and can activate one of
those credentials in the Windows ChatGPT/Codex app. A ChatGPT OAuth credential
is not a static token: Codex can refresh it and replace its refresh token.

Copying one `auth.json` into two independently running Codex homes creates two
writers. When either writer rotates the refresh token, the other copy can
become stale and later fail with:

```text
Your access token could not be refreshed because your refresh token was revoked.
```

The solution is a credential broker with one canonical identity record and one
refresh owner at a time. It is not an unrestricted two-way file watcher.

## Current Layout

The canonical profile inventory is in WSL:

```text
~/.config/workbenches/openai-profiles.json
~/.chatgpt-profiles/profiles/<family>/<category>/<profile>/auth.json
```

For example:

```text
~/.chatgpt-profiles/profiles/opensoft/max/max-001/auth.json
```

The Windows ChatGPT/Codex app has one active Codex home:

```text
C:\Users\<user>\.codex\auth.json
```

Windows does not have a separate profile inventory by default. The native
openProfiler app discovers WSL profiles and checks a selected credential into
the Windows active home.

## Incident Model

The `max-001` failure demonstrated the race:

1. WSL had a valid `max-001` credential.
2. The credential was copied into the Windows Codex home.
3. Windows refreshed it and received a new refresh-token generation.
4. WSL retained the older generation.
5. `pcodex max001` attempted to refresh with the revoked WSL generation.

The obsolete `opensoft-max-1` profile was unrelated. The failure came from two
copies of the same real account progressing independently.

## Architecture Decisions

### Keep one canonical credential authority

Each `IdentityProfile` has one canonical complete credential bundle. With the
current profile layout, the authority is the canonical WSL profile:

```text
~/.chatgpt-profiles/profiles/<profile-path>/auth.json
```

Windows and provider runtime homes contain checked-out working copies. They are
not independent authorities.

### Allow one refresh owner

For each ChatGPT account ID, exactly one runtime may own refresh rights:

```text
none | windows | wsl:<runtime-id>
```

The owner may make authenticated requests, refresh credentials, and publish a
new generation. Other copies must be dormant. A client that is running outside
the broker is an unmanaged writer and prevents a safe ownership transfer.

### Synchronize complete credential bundles

openProfiler copies the complete validated `auth.json`, including:

- `auth_mode`;
- access token;
- refresh token;
- ID token;
- ChatGPT account ID;
- refresh metadata.

It must never copy only an access token or merge fields from different files.

### Do not use modification time as authority

Filesystem modification time, access-token expiry, and `last_refresh` are
evidence, not authority. A stale process can write an older credential after a
newer file was installed.

An update is accepted when:

1. it comes from the current lease owner;
2. its account identity matches the selected profile;
3. the source file is stable throughout the snapshot;
4. the complete bundle passes structural validation.

If two non-identical credentials changed without an established owner,
openProfiler enters conflict state instead of selecting the newest timestamp.

### Never overwrite a running non-owner

Reading a stable snapshot from a running owner is allowed. Writing a credential
into a running target is not reliable because the process may cache its old
credential and later overwrite the new file.

Stopping Windows is therefore not required for a read-only Windows-to-WSL
snapshot. It is required when WSL becomes the new refresh owner because the
Windows app has no documented pause-refresh operation.

### Treat file watching as detection

Watchers detect that the owner may have rotated its credential. They do not
decide ownership.

Codex can replace `auth.json` atomically, so openProfiler watches the parent
directory, debounces events, reopens the file, and confirms that two successive
reads have the same hash before accepting the snapshot.

## System Model

```text
Canonical IdentityProfile
          |
          v
  openProfiler broker
      /         \
     v           v
Windows home   WSL runtime home
```

The broker is the only component allowed to:

- grant or release a refresh lease;
- activate a credential in a runtime home;
- accept a rotated generation;
- copy a generation to dormant mirrors;
- persist recovery metadata;
- declare and resolve conflicts.

Codex itself continues to own the managed ChatGPT OAuth refresh operation.
openProfiler coordinates where that operation is allowed to occur.

## Domain Model

### CredentialAuthority

Non-secret metadata describing the canonical identity:

```text
provider
identity_profile_id
expected_email
expected_account_id
authority_path
credential_store
accepted_generation
accepted_last_refresh
accepted_credential_hash
accepted_refresh_hash
owner
lease_id
lease_started_at
state
```

Hashes are comparison fingerprints, not authentication material. They must
still be treated as private operational metadata and not exposed in normal UI
or logs.

### CredentialCheckout

One activated working copy:

```text
target
path
identity_profile_id
account_id
generation_hash
process_id
process_started_at
last_observed_at
status
```

Targets include `windows` and a WSL runtime ID.

### CredentialLease

The single-writer grant:

```text
lease_id
account_id
owner
checkout_id
process_id
issued_at
heartbeat_at
expires_at
state
```

Lease states are `requested`, `active`, `releasing`, `released`, `stale`, and
`conflict`.

### CredentialEvent

A sanitized audit entry:

```text
event_id
account_id
identity_profile_id
event_type
source
previous_generation
new_generation
occurred_at
result
error_code
```

Events never contain token values, serialized credentials, command output, or
raw JWT claims.

## State Machine

```text
                 +-------------+
                 |    idle     |
                 +------+------+
                        |
              checkout(target)
                        |
                        v
              +---------+---------+
              | owner:<target>    |
              +----+----------+---+
                   |          |
             rotation      release
                   |          |
                   v          v
              publishing  reconciling
                   |          |
                   +----+-----+
                        |
                        v
                      idle

Any unowned divergence -> conflict
Interrupted operation  -> recovery_required
```

Only an active owner can transition through `rotation` and `publishing`.

## Credential Validation

Before accepting or activating a credential, the backend verifies:

1. the source and destination are regular files, not symlinks;
2. `auth_mode` is `chatgpt`;
3. the bundle contains access, refresh, and ID tokens;
4. the ID token email matches the profile's declared email;
5. the ChatGPT account ID matches the recorded account ID;
6. the source hash remains stable across the copy;
7. the destination hash equals the accepted source hash;
8. Unix mode is `0600`, or Windows ACLs remain restricted;
9. no credential value is returned to the webview or logs.

An access token that currently works is not sufficient proof that the refresh
token remains valid. After ownership transfer, openProfiler should use
`account/read` without forcing refresh, followed by a minimal real request when
the user requests end-to-end verification.

## Core Workflows

### Start Windows as owner

1. Resolve the selected canonical WSL profile.
2. Acquire the account lease for `windows`.
3. Confirm no WSL runtime owns or is using the account.
4. Validate the canonical credential.
5. Stop the Windows app if its active identity must be changed.
6. Preserve the existing credential rollback transaction.
7. Atomically install the canonical credential into the Windows Codex home.
8. Launch the Windows app.
9. Record its process and checkout generation.
10. Watch the Windows Codex home for credential replacement.

If Windows was already running as the verified owner, a read-only snapshot does
not require stopping it.

### Publish a Windows rotation

1. Detect replacement of the Windows `auth.json`.
2. Confirm Windows holds the active lease.
3. Wait for the file to become stable.
4. Validate identity and credential structure.
5. Compare it with the accepted generation.
6. Atomically replace the canonical WSL credential.
7. Verify the canonical destination hash and permissions.
8. Update non-secret generation metadata.
9. Update only dormant mirrors.

The running Windows credential remains the owner. WSL runtimes may not start
with this account until ownership transfers.

### Transfer ownership from Windows to WSL

1. Acquire a transfer transaction for the account.
2. Snapshot and validate the current Windows credential.
3. Stop the Windows app or otherwise prove it cannot make requests.
4. Re-read Windows after shutdown.
5. If the credential changed, accept the final stable owner generation.
6. Publish that generation to the canonical WSL profile.
7. Activate the canonical credential in the selected WSL history runtime.
8. Start Codex and verify the expected account.
9. Grant the lease to the WSL runtime.

The stop occurs for ownership transfer, not because reading the Windows
credential inherently requires shutdown.

### Run `pcodex`

The profile launcher should use the broker:

```text
pcodex max001
  -> resolve canonical profile
  -> acquire WSL lease
  -> reconcile previous owner
  -> launch Codex
  -> observe credential changes
  -> check in final credential
  -> release lease
```

If Windows currently owns the account, the launcher blocks with an actionable
message rather than starting with a duplicated refresh token.

### Publish a WSL rotation

1. Detect replacement of the owning WSL runtime's `auth.json`.
2. Validate identity, structure, ownership, and file stability.
3. Publish the accepted generation to the canonical identity profile.
4. Copy it to Windows only when the Windows app is stopped.
5. Otherwise mark the Windows checkout stale and update it at its next
   activation.

### Release WSL ownership

1. Wait for Codex to become idle and exit.
2. Read the final WSL credential.
3. Publish any accepted rotation to the authority.
4. Update dormant mirrors.
5. Remove temporary runtime credentials where applicable.
6. Release the lease.

### Snapshot without ownership transfer

openProfiler may copy the current owner credential to the canonical authority
while the owner remains running when:

- the source is stable across the snapshot;
- the owner lease is valid;
- no other runtime is permitted to use the copied refresh token;
- the copy is treated as a dormant recovery mirror.

This is how Windows rotations can continuously update WSL without stopping
Windows. It does not authorize concurrent `pcodex` use.

## Conflict Handling

A conflict exists when:

- Windows and WSL both changed while neither had a valid lease;
- a non-owner writes a different refresh generation;
- account identity differs from the selected profile;
- the source changes during snapshot;
- the broker restarts with multiple apparently active writers;
- a stale process writes after an ownership transfer.

Resolution rules:

1. Stop or quiesce all writers.
2. Never select by modification time alone.
3. Prefer the final credential from the recorded lease owner.
4. Verify identity before installing it anywhere.
5. Preserve restricted rollback copies until validation succeeds.
6. If ownership cannot be established, require a fresh managed ChatGPT login.

The broker must never repeatedly test competing refresh tokens. Refreshing one
candidate can invalidate another and make diagnosis less reliable.

## Process and IPC Architecture

The native openProfiler process should own the broker state because it already
controls the packaged Windows app and can access registered WSL profile stores.

The WSL `pcodex` wrapper needs a narrow authenticated control interface:

```text
credential/status
credential/lease/acquire
credential/lease/heartbeat
credential/publish
credential/lease/release
```

The transport can be an authenticated loopback endpoint or another
cross-Windows/WSL local IPC mechanism. Requirements:

- loopback only;
- high-entropy capability credential stored with restricted Windows ACLs;
- no raw OAuth credential in RPC messages;
- broker reads credentials directly from approved paths;
- request IDs and idempotency for mutating operations;
- bounded timeouts and stale-lease recovery;
- no generic filesystem or command-execution endpoint.

Until the broker is available, launchers should warn when the same account is
active in Windows and WSL. openProfiler cannot guarantee safety for clients
started outside its lifecycle.

## Atomic File Operations

Credential publication uses:

1. an approved source path;
2. a temporary file in the destination directory;
3. restrictive permissions before writing;
4. full write and file flush;
5. source stability recheck;
6. atomic rename over the destination;
7. destination-directory flush where supported;
8. post-write hash and identity verification.

Windows activation preserves the destination ACL. Unix profile credentials use
mode `0600`. Symlinked credential files and active homes are rejected.

## SOPS Escrow

SOPS is a recovery snapshot, not the live synchronization channel or lease
authority.

An escrow policy must account for refresh-token rotation:

- never restore escrow over a newer accepted live generation;
- update escrow only after a generation is accepted by the broker;
- debounce automatic escrow updates to avoid unnecessary repository churn;
- verify decrypt-and-hash round trips without printing plaintext;
- record only non-secret account and generation metadata;
- treat an old escrow refresh token as potentially revoked;
- require login when a restored credential cannot refresh.

For reliable cross-workstation concurrent use, a workstation-local broker is
insufficient. That requires a remote credential authority and distributed
lease. Copying one refresh token from SOPS or Key Vault to multiple active
workstations recreates the same race.

## Security Requirements

- Credentials never enter React state or Tauri event payloads.
- Token values and raw JWTs never appear in logs.
- Account email and account ID are used only for identity verification.
- Credential fingerprints are hidden from normal UI.
- Rollback credentials remain mode `0600` or under restricted Windows ACLs.
- Watch the parent directory to detect atomic file replacement.
- Do not follow symlinks when resolving credential sources or destinations.
- Refuse account mismatches even when the access token currently succeeds.
- Do not copy into a running non-owner process.
- Fail closed when lease ownership is uncertain.
- Keep a bounded sanitized audit trail for recovery and support.

## Delivery Plan

### Phase 1: Credential inventory and diagnostics

- Add non-secret credential metadata and duplicate-account detection.
- Show canonical authority, active checkout, owner, and stale mirrors.
- Detect Windows/WSL divergence without modifying credentials.

### Phase 2: Atomic snapshot and publication

- Implement stable snapshot, validation, atomic write, and rollback.
- Publish rotations from a known owner to the canonical profile.
- Add explicit Windows-to-WSL and WSL-to-Windows reconciliation commands.

### Phase 3: Single-writer leases

- Add per-account lease state and process heartbeats.
- Integrate Windows lifecycle management.
- Add WSL broker IPC and `pcodex` wrapper integration.
- Block concurrent refresh writers.

### Phase 4: Automatic rotation observation

- Watch owner credential directories.
- Publish accepted rotations to the authority.
- Mark running or unavailable mirrors stale instead of overwriting them.
- Add crash recovery and conflict UI.

### Phase 5: Escrow policy

- Trigger encrypted backup after accepted rotation according to policy.
- Validate registry round trips.
- Add optional remote authority design for multiple workstations.

## Test Strategy

Unit tests:

- complete bundle validation;
- account and email mismatch rejection;
- source stability checks;
- generation comparison;
- owner-only publication;
- lease expiry and recovery;
- conflict transitions;
- path and symlink rejection;
- sanitized event serialization.

Integration tests:

- Windows rotation publishes to dormant WSL authority;
- WSL rotation updates Windows only while Windows is stopped;
- copying into a running non-owner is refused;
- stale process write after transfer enters conflict;
- interrupted atomic publication preserves one complete credential;
- owner process exits during refresh;
- broker restart with a stale lease;
- real Codex status and inference after controlled transfer.

Regression scenario:

1. seed Windows and WSL with one generation;
2. rotate Windows to a second generation;
3. confirm WSL is marked stale;
4. publish Windows to canonical WSL;
5. transfer ownership to WSL;
6. confirm the old WSL generation can never overwrite the accepted generation.

## Non-Goals

- Concurrent independent refresh writers for one ChatGPT account.
- Selecting credential authority by modification time.
- Sharing only an access token.
- Editing credentials in a running non-owner.
- Treating SOPS or Key Vault as an OAuth refresh broker.
- Managing browser cookies or ChatGPT web sessions.
- Using experimental externally managed ChatGPT tokens in the first release.

## References

- [Codex authentication](https://learn.chatgpt.com/docs/auth)
- [Codex App Server](https://learn.chatgpt.com/docs/app-server)
- [Codex environment variables](https://learn.chatgpt.com/docs/config-file/environment-variables)
- [Windows app: share config, auth, and sessions with WSL](https://learn.chatgpt.com/docs/windows/windows-app#share-config-auth-and-sessions-with-wsl)
