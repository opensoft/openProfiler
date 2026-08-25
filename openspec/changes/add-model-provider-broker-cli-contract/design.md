## Context

openProfiler currently discovers and activates local Codex and Claude profile
credentials through a Tauri desktop application. It has no non-interactive
broker command, no credential-reference protocol, and no token-mint response
owned by this repository. openxFactory has a server-side provider seam that is
prepared to invoke a declared command, but its paused adapter prototype cannot
become the authority for openProfiler's interface.

The boundary carries raw API keys during enrollment and short-lived provider
tokens during minting. It therefore needs a closed wire shape, bounded process
I/O, explicit token semantics, and fixtures that never contain a real secret.

## Goals / Non-Goals

**Goals:**

- Own a versioned non-interactive broker protocol in openProfiler.
- Keep long-lived credential material in the broker and give consumers only an
  opaque reference.
- Mint provider-native bearer capability with a bounded lifetime, explicit
  provider-native scopes, auditable issuance metadata, and routing metadata.
- Make the first consumer testable without provider credentials or network
  access.
- Keep every secret out of arguments, environment variables, logs, errors,
  files outside the future vault backend, and consumer responses.

**Non-Goals:**

- Implement a production credential vault or OAuth provider enrollment.
- Add provider-specific OAuth flows to the desktop application.
- Normalize arbitrary OpenAI, Anthropic, or other provider APIs in
  openProfiler.
- Put openProfiler in the model request path.
- Change existing profile discovery or activation behavior.

## Decisions

### 1. A separate `openprofiler-broker` binary owns automation

The official invocation is `openprofiler-broker stdio`. It is separate from
the Tauri desktop executable so a server process does not initialize a webview
or inherit desktop permissions. The executable accepts no credential in argv
or the environment.

Alternative considered: add hidden flags to the desktop binary. Rejected
because desktop initialization and permissions would become part of the
automation boundary.

### 2. One process handles exactly one bounded JSON request

The command reads one UTF-8 JSON object from standard input, to EOF, with a
262,144-byte maximum. It writes one UTF-8 JSON object to standard output and
exits. Success exits zero; refusal exits non-zero. Standard error contains no
request, credential, token, provider body, or subprocess detail.

Every request and response carries `schema_version: 1`, `operation`, and a
closed operation-specific shape. One request per process bounds secret
lifetime and avoids stateful framing or multiplexing.

Alternative considered: a long-running JSON-lines daemon. Rejected for v1
because it retains secrets and minted tokens in a larger process lifetime and
requires cancellation, framing, and concurrency semantics the consumer does
not need.

### 3. API-key custody and OAuth enrollment have different entrances

`store_credential` accepts raw material only when `auth_kind` is `api_key` and
returns an opaque `credential_ref`. For `oauth`, enrollment belongs to
openProfiler's own interactive surface; consumers may bind the resulting
reference but must never submit an authorization code, access token, or refresh
token to this command.

Alternative considered: accept OAuth tokens through the same password field.
Rejected because it bypasses the broker-owned redirect, consent, refresh, and
revocation lifecycle.

### 4. A mint is provider-native, scoped, auditable, and short-lived

`mint` returns:

- `token_type: "provider_native_bearer"`;
- a non-empty bearer `token`;
- integer Unix `expires_at`, later than issuance and no more than 3,600 seconds
  in the future;
- a non-empty, duplicate-free `scope` array of opaque provider-native strings;
- `grant` metadata carrying `issued_by`, `approved_by`, and `audit_ref`;
- provider metadata carrying `protocol: "xfactory_workbench_v1"`, catalog and
  completion endpoints, and a bounded timeout.

The consumer may use the token only with the returned provider metadata, must
discard it at expiry, and must not retry the expired call automatically. A new
later operation mints a new capability.

Alternative considered: broker-issued tokens accepted by an openProfiler
proxy. Rejected because it puts the broker back in the request path, contrary
to the governing latency decision.

### 5. Routing metadata is constrained before a token can be sent

Endpoints must use HTTPS. HTTP is allowed only for literal loopback hosts
(`localhost`, `127.0.0.0/8`, or `::1`) to support deterministic local tests.
User information in URLs is forbidden. The v1 protocol identifier is closed so
a consumer cannot silently interpret an unknown provider response shape.

### 6. The first shipped implementation may be honestly unconfigured

This change may ship the parser, schemas, examples, conformance fixture, and a
production command that returns the fixed `broker_unconfigured` refusal until
a vault backend is installed. It must not emulate custody by writing secrets to
the profile store or repository.

## Risks / Trade-offs

- **The declared protocol lands before a production vault.** → The command
  fails closed with `broker_unconfigured`; fixtures are explicitly test-only.
- **Provider-native scopes are opaque to openProfiler.** → The broker returns
  them verbatim and the consumer never infers authority from spelling.
- **A compromised broker can return a hostile endpoint.** → Consumers enforce
  the HTTPS/loopback URL rule and the closed protocol identifier before adding
  authorization.
- **Process arguments can still name an attacker-controlled executable.** →
  The consumer treats broker argv as privileged local configuration and never
  derives it from provider responses or catalog data.
- **The one-hour maximum may be too short for a future provider.** → Widening
  the limit requires a versioned contract change rather than consumer drift.

## Migration Plan

1. Ship schemas, examples, conformance tests, and the fail-closed command.
2. Update openxFactory's adapter to require the v1 response fields and use the
   official command through its declared argv binding.
3. Add a future vault/OAuth implementation behind the same contract.
4. Roll back by removing broker configuration; consumers return to their
   existing unconfigured posture without touching profile activation.

## Open Questions

None for protocol v1. Provider-specific vault and OAuth implementations require
their own governed changes.
