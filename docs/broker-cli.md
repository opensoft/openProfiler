# openProfiler credential broker CLI

`openprofiler-broker` is openProfiler's credential-custody and token-minting
surface. It holds long-lived provider credentials on the local device and mints
short-lived, scoped tokens on demand. It is **not** a proxy: nothing it issues
puts it in the request path between a consumer and a model provider.

This document is the **declaration** the consuming binding reads. A consumer —
the xFactory ideation dashboard (doxBench) is the first — stores a binding that
names this program and the argv template below, hands secrets to its standard
input, and reads its standard output. When this surface changes, the binding
changes and no consumer code does.

- Status of each part is stated inline: **IMPLEMENTED** or **DECLARED-DESIGN**.
- Lane context: openxFactory `openspec/changes/add-model-provider-broker`.
- Accountability shape: openxFactory `openspec/specs/credential-contracts`.

## Contents

- [The boundary](#the-boundary)
- [What is implemented and what is staged](#what-is-implemented-and-what-is-staged)
- [Custody store](#custody-store)
- [CLI surface](#cli-surface)
  - [`intake`](#intake)
  - [`mint`](#mint)
  - [`revoke`](#revoke)
  - [`list`](#list)
  - [`authorize`](#authorize-declared-design)
- [Exit codes](#exit-codes)
- [What a minted token carries](#what-a-minted-token-carries)
- [The audit record](#the-audit-record)
- [Mid-turn expiry: re-mint and retry once](#mid-turn-expiry-re-mint-and-retry-once)
- [The OAuth authorization flow](#the-oauth-authorization-flow-declared-design)
- [Consumer binding template](#consumer-binding-template)
- [Staged follow-ups](#staged-follow-ups)

## The boundary

The broker holds custody; the consumer calls the provider.

```text
human ──secret on stdin──▶ openprofiler-broker intake ──▶ custody store (0600)
                                                  │
consumer  ◀──reference on stdout──────────────────┘

consumer ──mint --reference──▶ openprofiler-broker mint ──▶ token + expires_at
                                                  │            + audit_ref
consumer ──token──────────────────────────────────┴──────────▶ model provider
```

Three properties hold by construction:

1. **A secret never appears in argv.** Every command that accepts a secret reads
   it from standard input. There is no `--secret`, `--api-key` or `--token`
   flag; passing one is a usage error, because process argv is world-readable on
   the platforms openProfiler runs on.
2. **A secret is written to exactly one file.** The custody entry, mode `0600`,
   under a mode-`0700` custody root. Nothing else — no index, no audit record,
   no log line, no error message, no temporary file that survives — ever holds
   credential material.
3. **The broker is never in the request path.** It answers `mint` from local
   custody (`api_key`) or from one provider refresh call (`oauth`), and then it
   is done. The provider call itself is the consumer's, made with the minted
   token directly.

## What is implemented and what is staged

| Capability                                   | Auth kind | Status                                                          |
| -------------------------------------------- | --------- | --------------------------------------------------------------- |
| `intake` → custody → reference               | `api_key` | **IMPLEMENTED**                                                 |
| `mint` → token + `expires_at` + `audit_ref`  | `api_key` | **IMPLEMENTED**                                                 |
| `revoke` → destroys custody, keeps audit     | `api_key` | **IMPLEMENTED**                                                 |
| `list` → non-secret references               | both      | **IMPLEMENTED**                                                 |
| Append-only audit record per event           | both      | **IMPLEMENTED**                                                 |
| Restricted modes (`0600` files, `0700` dirs) | both      | **IMPLEMENTED** (Unix; Windows inherits the directory ACL)      |
| `intake` of an OAuth grant                   | `oauth`   | **REFUSED** — exit 5, points at `authorize`                     |
| `authorize` browser flow                     | `oauth`   | **DECLARED-DESIGN** — surface accepted, exit 5                  |
| `mint` from a stored OAuth grant             | `oauth`   | **DECLARED-DESIGN** — library path present, refuses with exit 5 |
| OS keyring custody                           | both      | **STAGED** — see [Staged follow-ups](#staged-follow-ups)        |

An honest refusal with a documented exit code is the contract for everything
marked DECLARED-DESIGN. The consumer can therefore ship its OAuth binding today
and get a distinguishable "not built yet" rather than a plausible-looking lie.

## Custody store

Resolved from `OPENPROFILER_BROKER_HOME`, defaulting to `~/.openprofiler/broker`.

```text
<broker home>/                       directory, mode 0700
├── custody/                         directory, mode 0700
│   └── <reference>.json             file, mode 0600   ← the ONLY secret-bearing file
├── references.json                  file, mode 0600   non-secret index (what `list` reads)
└── audit/
    └── broker-audit.jsonl           file, mode 0600   non-secret, append-only
```

`references.json` and `broker-audit.jsonl` carry no credential material and are
safe to read, quote and diff. They are held at `0600` anyway, because who holds
which provider binding is not something to volunteer to other local accounts.

Writes are atomic: content goes to a mode-`0600` temporary file beside the
target, is fsynced, and is renamed over it. This is the same discipline
`activate_profile` uses for provider credentials, and it is deliberate reuse
rather than a second pattern.

`list` never opens a custody file. Reading a reference index is not a reason to
page a secret into this process's memory.

**Identifiers.** A reference is `opref-` plus 24 lowercase hex characters; an
audit reference is `opaud-` plus 24. They are unique, not unguessable: they are
derived from a monotonic clock, the process id and a per-process counter, not
from a CSPRNG. That is stated rather than glossed, because a reference is not a
capability — it names an entry that only the local user's own file permissions
protect, and guessing one grants nothing. Every reference accepted on argv is
validated against that exact shape before it is joined to a path, so a
reference can never traverse out of the custody root.

## CLI surface

```text
openprofiler-broker <command> [options]

Commands:
  intake      take a secret on stdin into custody and print a reference
  mint        issue a short-lived token for a held credential
  revoke      destroy a held credential
  list        print the non-secret reference index
  authorize   begin a provider OAuth authorization  (DECLARED-DESIGN)
  help        print usage
```

Also accepted: `--help` / `-h` (usage on stdout, exit 0) on the program and on
any subcommand, and `--version` / `-V` (the crate version on stdout, exit 0).

**Flag grammar.** Both `--name value` and `--name=value` are accepted. Every
value is named by a flag; there are no positional arguments. A flag given twice
is an error unless it is documented as repeatable (only `--scope` is). An
argument that is not valid Unicode is refused rather than lossily decoded.

**Output discipline, for every command.**

- On success: exactly one JSON object, on **stdout**, followed by a newline.
  Exit code `0`. Nothing else is ever written to stdout.
- On failure: exactly one JSON object, on **stderr**, followed by a newline, and
  a non-zero exit code. **stdout is empty.**
- The exit code is authoritative. A consumer that cannot parse stdout should
  treat the invocation as failed regardless of what the exit code said.
- Every JSON object carries `schema_version` and `kind`, matching the workspace
  convention that every record declares its envelope.
- No output on either stream ever contains credential material. Error messages
  are fixed strings plus non-secret identifiers.

### `intake`

**IMPLEMENTED** for `--auth-kind api_key`. Refuses `oauth` with exit `5`.

```text
openprofiler-broker intake
    --binding <binding-id>
    --provider <provider>
    --auth-kind api_key|oauth
    --approved-by <principal>
    [--label <text>]
    [--issued-by <principal>]
    [--lifetime-seconds <30..3600>]
```

| Option               | Required | Meaning                                                                                                                                                                             |
| -------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--binding`          | yes      | The consumer's own binding id, carried into every audit record so the two sides correlate. `[A-Za-z0-9._:-]`, 1–128 chars.                                                          |
| `--provider`         | yes      | Provider name. Domain content, deliberately not enumerated by the broker. `[A-Za-z0-9._:-]`, 1–64 chars.                                                                            |
| `--auth-kind`        | yes      | `api_key` or `oauth`.                                                                                                                                                               |
| `--approved-by`      | yes      | The human principal who supplied the credential. Required because `credential-contracts` holds that a grant without an approver is invalid, and intake is where an approver exists. |
| `--label`            | no       | Human-facing label. Free text, no control characters, ≤ 256 chars.                                                                                                                  |
| `--issued-by`        | no       | Defaults to `openprofiler-broker/<version>`.                                                                                                                                        |
| `--lifetime-seconds` | no       | The **maximum** lifetime this credential may ever mint. Default `300`, clamped to `30..=3600`.                                                                                      |

**stdin**: the secret, read to EOF, at most 64 KiB. A single trailing `\n` or
`\r\n` is trimmed; nothing else is. It must be valid UTF-8. Standard input is
read exactly once and never echoed, and it is not read at all when the
invocation is refused before it — an `--auth-kind oauth` intake never touches
it.

| Refusal                           | Exit | `code`                    |
| --------------------------------- | ---- | ------------------------- |
| Empty standard input              | `2`  | `empty_secret`            |
| Standard input is not valid UTF-8 | `2`  | `invalid_secret_encoding` |
| Larger than 64 KiB                | `2`  | `usage`                   |
| `--auth-kind oauth`               | `5`  | `not_implemented`         |

**stdout**:

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_intake",
  "reference": "opref-4f2a91c07be3d5a8140b6e77",
  "binding": "anthropic-default",
  "provider": "anthropic",
  "auth_kind": "api_key",
  "label": "Anthropic — team key",
  "created_at": "2026-08-26T14:03:11Z",
  "max_lifetime_seconds": 300,
  "issued_by": "openprofiler-broker/0.1.4",
  "approved_by": "brett@opensoft.one",
  "audit_ref": "opaud-9c31e0b57af2648d0d13a5ce"
}
```

`label` is `null` when none was given. The secret appears nowhere in this
object, and the consumer is expected to store exactly this — a binding.

### `mint`

**IMPLEMENTED** for a held `api_key` credential. A held `oauth` grant refuses
with exit `5`.

```text
openprofiler-broker mint
    --reference <opref-…>
    [--scope <scope>]...
    [--lifetime-seconds <30..3600>]
    [--retry-of <opaud-…>]
```

| Option               | Required | Meaning                                                                                                                                                                                                                                 |
| -------------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--reference`        | yes      | The reference `intake` returned. A value that is not reference-shaped is a **usage** error (exit `2`), distinct from a well-formed reference nothing is held under (exit `3`) — a typo and an absent credential are different problems. |
| `--scope`            | no       | Repeatable, up to 32 times. Recorded on the token and in the audit record. `[A-Za-z0-9._:/-]`, 1–128 chars each. See the enforcement note below.                                                                                        |
| `--lifetime-seconds` | no       | Requested lifetime. The effective lifetime is `min(requested, max_lifetime_seconds)`; a larger request is clamped rather than refused, and the response states the effective value. Defaults to the credential's maximum.               |
| `--retry-of`         | no       | The `audit_ref` of a mint this one replaces, for the mid-turn re-mint rule below. Recorded, so a retry is visible in the audit trail rather than indistinguishable from an unrelated second mint.                                       |

**stdin**: not read. The caller may close it.

**stdout**:

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_mint",
  "reference": "opref-4f2a91c07be3d5a8140b6e77",
  "binding": "anthropic-default",
  "provider": "anthropic",
  "auth_kind": "api_key",
  "token": "<provider-native credential>",
  "token_type": "api_key",
  "issued_at": "2026-08-26T14:07:52Z",
  "expires_at": "2026-08-26T14:12:52Z",
  "expires_in_seconds": 300,
  "scope": ["messages:write"],
  "issued_by": "openprofiler-broker/0.1.4",
  "approved_by": "brett@opensoft.one",
  "audit_ref": "opaud-1b6d24fe90c3a7550e2f8813",
  "retry_of": null,
  "enforcement": { "expiry": "broker_bookkeeping", "scope": "declared" }
}
```

`token` is the only secret-bearing field the broker ever emits, and it is
emitted only here. The consumer holds it in process memory, uses it against the
provider directly, and lets it die with the request. It is never written to a
file, placed in a response to a browser, or logged.

### `revoke`

**IMPLEMENTED**.

```text
openprofiler-broker revoke --reference <opref-…>
```

Destroys the custody entry and removes the index entry. The audit trail is
**not** removed — revocation appends to it. An unknown reference exits `3`;
revoking is not idempotent-by-silence, because "there was nothing there" and "it
is gone now" are different answers to the same question.

**stdin**: not read.

**stdout**:

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_revocation",
  "reference": "opref-4f2a91c07be3d5a8140b6e77",
  "binding": "anthropic-default",
  "provider": "anthropic",
  "auth_kind": "api_key",
  "revoked": true,
  "revoked_at": "2026-08-26T14:20:03Z",
  "audit_ref": "opaud-77b0c4e91d3a5628ff0e1a42"
}
```

### `list`

**IMPLEMENTED**.

```text
openprofiler-broker list
```

**stdin**: not read.

**stdout**:

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_reference_list",
  "references": [
    {
      "reference": "opref-4f2a91c07be3d5a8140b6e77",
      "binding": "anthropic-default",
      "provider": "anthropic",
      "auth_kind": "api_key",
      "label": "Anthropic — team key",
      "created_at": "2026-08-26T14:03:11Z",
      "max_lifetime_seconds": 300,
      "issued_by": "openprofiler-broker/0.1.4",
      "approved_by": "brett@opensoft.one",
      "mint_count": 3,
      "last_minted_at": "2026-08-26T14:07:52Z"
    }
  ]
}
```

`references` is `[]` when the store is empty or absent — an empty store is not
an error. Entries are ordered by `created_at`, then by reference.

### `authorize` (DECLARED-DESIGN)

The surface is accepted and validated; the flow is not built. It refuses with
exit `5` and code `oauth_not_implemented`.

```text
openprofiler-broker authorize
    --binding <binding-id>
    --provider <provider>
    --approved-by <principal>
    [--label <text>]
    [--scope <scope>]...
    [--lifetime-seconds <30..3600>]
```

## Exit codes

| Code | `code` field              | Meaning                                                                                                                                                                                                                         |
| ---- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0`  | —                         | Success. One JSON object on stdout.                                                                                                                                                                                             |
| `1`  | `internal`                | Unexpected internal failure, including a standard output that could not be written.                                                                                                                                             |
| `2`  | `usage`                   | Malformed invocation: unknown command, unknown or repeated flag, missing required flag, invalid value, a positional argument, a malformed reference.                                                                            |
| `2`  | `secret_in_argv`          | A flag that would carry credential material on argv — `--secret`, `--api-key`, `--apikey`, `--key`, `--token`, `--access-token`, `--refresh-token`, `--credential`, `--password`, `--passphrase`. Refused on **every** command. |
| `2`  | `empty_secret`            | Standard input held nothing where a secret was required.                                                                                                                                                                        |
| `2`  | `invalid_secret_encoding` | The credential on standard input is not valid UTF-8.                                                                                                                                                                            |
| `3`  | `unknown_reference`       | No credential is held under that reference.                                                                                                                                                                                     |
| `4`  | `credential_unusable`     | The reference resolves but the credential cannot be used — a corrupt custody entry, or a stored kind this build cannot mint.                                                                                                    |
| `5`  | `not_implemented`         | A declared surface that is not built: every OAuth path today.                                                                                                                                                                   |
| `6`  | `custody_unavailable`     | The custody store could not be read or written — permissions, a symlinked path the broker refuses to follow, a full disk.                                                                                                       |

Error body, on stderr:

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_error",
  "code": "unknown_reference",
  "message": "no credential is held under that reference",
  "exit_code": 3
}
```

For the consumer, everything non-zero maps onto its own fixed redacted refusal.
The codes exist so a human reading the console can tell "nothing is configured"
from "it broke" — which the dashboard spec requires of the refusal it renders.

## What a minted token carries

**It is provider-native, in both kinds.** A minted token is something the
provider itself accepts. The broker does not issue a credential of its own that
a provider was pre-configured to trust: that design would put the broker back in
the request path it was deliberately taken out of, and would make openProfiler a
component of every provider call's availability.

That commitment has an uncomfortable consequence for API keys, and this
document states it rather than burying it.

### `api_key` — the key itself, with broker-side bookkeeping

The minted token **is the stored API key, verbatim**. There is no provider
mechanism to hand out a shortened-life derivative of a plain API key: the
provider issued one long-lived bearer secret and knows nothing about openProfiler.

So `expires_at` on an `api_key` mint is **broker bookkeeping that the consumer
honors**, not an expiry the provider enforces. The field is real and it is
binding on the consumer — the dashboard spec already requires that a token not
be used past the declared expiry, and that it be discarded rather than
retried — but a copy of that token that escaped the consumer's process would
keep working until the key is revoked at the provider or with `revoke` here.

`enforcement` says so in machine-readable form, so a consumer never has to infer
it:

```json
"enforcement": { "expiry": "broker_bookkeeping", "scope": "declared" }
```

`scope` is likewise **declared, not enforced**: the scopes recorded on the mint
and in the audit record describe what the token was issued _for_. A plain API
key carries whatever authority the provider granted it, and the broker cannot
narrow that. Narrowing it is the provider's job, done by provisioning a
narrower key and taking _that_ into custody.

What the mint genuinely buys on this path is therefore custody and
accountability, and it is worth being precise about which: the long-lived secret
lives in one place with restricted modes instead of in the consumer's settings;
the consumer holds a reference it can safely log and commit; every issuance is an
audited event with an issuer, an approver, an expiry and an audit reference; and
revocation is one local command that stops future mints immediately.

### `oauth` — a provider-issued access token (DECLARED-DESIGN)

The minted token is the **short-lived access token the provider returns** when
the broker refreshes the stored grant. It is genuinely short-lived: `expires_at`
is the provider's own `expires_in` applied to the moment of the refresh, and the
provider stops accepting the token at that instant whoever holds it. `scope` is
the scope the grant carries, enforced by the provider.

```json
"enforcement": { "expiry": "provider", "scope": "provider" }
```

`token_type` is `bearer`, and the long-lived refresh grant never leaves custody.

This path is **not built**. `mint` against a stored OAuth grant refuses with exit
`5`. The refresh-token exchange is the named follow-up below.

### The two kinds side by side

|                    | `api_key`                             | `oauth`                      |
| ------------------ | ------------------------------------- | ---------------------------- |
| Minted material    | the stored key, verbatim              | provider-issued access token |
| `token_type`       | `api_key`                             | `bearer`                     |
| Provider-native    | yes                                   | yes                          |
| Expiry enforced by | **the consumer, on trust**            | the provider                 |
| Scope enforced by  | the provider's own key scope          | the provider, per grant      |
| Default lifetime   | 300 s (`--lifetime-seconds`, 30–3600) | the provider's `expires_in`  |
| Network per mint   | none                                  | one refresh call             |
| Status             | IMPLEMENTED                           | DECLARED-DESIGN              |

## The audit record

Every `intake`, `mint` and `revoke` appends one JSON object — one line — to
`<broker home>/audit/broker-audit.jsonl`. The file is append-only by convention
and by code path: the broker opens it for append and writes a single line; it
has no rewrite path.

The mint is the auditable event. Provider calls made under one minted token are
its children, and the broker neither sees nor records them.

```json
{
  "schema_version": 1,
  "kind": "openprofiler_broker_audit_record",
  "audit_ref": "opaud-1b6d24fe90c3a7550e2f8813",
  "event": "mint",
  "recorded_at": "2026-08-26T14:07:52Z",
  "reference": "opref-4f2a91c07be3d5a8140b6e77",
  "binding": "anthropic-default",
  "provider": "anthropic",
  "auth_kind": "api_key",
  "scope": ["messages:write"],
  "issued_by": "openprofiler-broker/0.1.4",
  "approved_by": "brett@opensoft.one",
  "expires_at": "2026-08-26T14:12:52Z",
  "enforcement": { "expiry": "broker_bookkeeping", "scope": "declared" },
  "retry_of": null
}
```

The four fields `credential-contracts` requires of a grant are all present and
all mandatory:

| Field         | Source                                                                                         |
| ------------- | ---------------------------------------------------------------------------------------------- |
| `issued_by`   | `--issued-by` at intake, defaulting to `openprofiler-broker/<version>`                         |
| `approved_by` | `--approved-by` at intake — required, never defaulted                                          |
| `expires_at`  | the minted token's expiry; on `intake` and `revoke`, `null` — no token exists at those moments |
| `audit_ref`   | this record's own identifier, returned on stdout by the command that caused it                 |

**No token material is recorded.** Not the token, not a prefix, not a hash. A
mint is identified by its `audit_ref`, which the consumer already holds.

## Mid-turn expiry: re-mint and retry once

Ruled by Brett, 2026-08-26: when a minted token expires part-way through a turn,
the consumer **re-mints and retries once**, and the retry is visibly recorded.

The broker's obligations under that rule:

- **A mint is cheap.** On the `api_key` path it is a local read and two small
  local writes — no network, no provider round trip. On the `oauth` path it is
  one refresh call.
- **A mint is repeatable.** Nothing about minting consumes the credential or
  changes what a subsequent mint returns. There is no per-credential mint limit.
- **A retry is visible.** The consumer passes the expired mint's `audit_ref` as
  `--retry-of`. The new audit record carries it, so the trail shows one turn
  that needed two tokens rather than two unrelated issuances.

A consumer that retries more than once against the same turn is outside this
rule, and the trail will show it.

## The OAuth authorization flow (DECLARED-DESIGN)

**Recommended and declared: the browser flow, with openProfiler as the local
redirect target.** The consumer hands off and never sees the grant.

This is the natural fit for a local-first desktop application, and the reasons
are structural rather than aesthetic:

- The refresh grant is the long-lived secret. Custody of long-lived secrets is
  the whole job openProfiler was given; routing the grant through the consumer
  first would hand it, however briefly, to the component that is supposed never
  to hold one.
- openProfiler already runs on the user's device with a browser available and a
  loopback interface it can bind. A localhost redirect is the standard public
  client pattern, and needs no hosted callback and no client secret.
- The consumer's own settings surface can neither complete nor observe the flow,
  which is exactly the property that makes the resulting binding safe to log.

Declared shape:

1. `openprofiler-broker authorize --binding … --provider … --approved-by …`
   binds `127.0.0.1` on an ephemeral port and derives a PKCE verifier and
   challenge (S256).
2. It prints the authorization URL on stdout as a JSON object of kind
   `openprofiler_broker_authorization_started`, and opens the user's browser at
   that URL. The user authenticates with the provider; openProfiler is not in
   that exchange.
3. The provider redirects to
   `http://127.0.0.1:<ephemeral-port>/openprofiler/oauth/callback`. The listener
   accepts exactly one request, matches the `state` value, and closes.
4. The broker exchanges the code plus the PKCE verifier for a refresh grant,
   writes the grant into custody at mode `0600`, and appends an `intake` audit
   record.
5. It prints the same `openprofiler_broker_intake` object `intake` prints, with
   `auth_kind` of `oauth`. From that point `mint` behaves as the `oauth` row
   above describes.

Provider client ids, authorization endpoints and token endpoints are per-provider
configuration, not code, for the same reason the consumer's broker invocation is
configuration: openProfiler must not freeze an endpoint it cannot verify.

## Consumer binding template

What the openxFactory `model-provider-binding` should declare. No field holds a
secret — the shape has no secret field at all, so no code path can populate one.

```yaml
schema_version: 1
kind: model_provider_binding
id: anthropic-default
label: "Anthropic — team key"
provider: anthropic
auth_kind: api_key # api_key | oauth
credential_reference: opref-4f2a91c07be3d5a8140b6e77
broker_invocation:
  program: openprofiler-broker # resolved on PATH, or an absolute path
  timeout_seconds: 10
  intake_argv:
    - intake
    - --binding
    - "{binding_id}"
    - --provider
    - "{provider}"
    - --auth-kind
    - "{auth_kind}"
    - --label
    - "{label}"
    - --approved-by
    - "{approved_by}"
    - --lifetime-seconds
    - "{max_lifetime_seconds}"
  mint_argv:
    - mint
    - --reference
    - "{credential_reference}"
    - --lifetime-seconds
    - "{requested_lifetime_seconds}"
  revoke_argv:
    - revoke
    - --reference
    - "{credential_reference}"
  list_argv:
    - list
```

The mid-turn retry appends `--retry-of {previous_audit_ref}` to `mint_argv`.

Contract for the consumer, restated as obligations:

- Write the secret to the child's stdin, then **close stdin**. `intake` reads to
  EOF and will otherwise wait.
- Never place a secret in `argv`, an environment variable, or a file. Env vars
  are inherited by every descendant; argv is world-readable.
- Parse stdout as one JSON object. Treat a non-zero exit, unparseable stdout, or
  a timeout identically: no token could be minted.
- Hold the minted `token` in process memory only, honor `expires_at`, and
  discard rather than reuse past it.
- Store the returned `reference` and nothing else.

## Staged follow-ups

Named, in the order they should be taken:

1. **OAuth refresh exchange** — the `mint` path for a stored grant: refresh
   against the provider's token endpoint, return the access token with
   `enforcement.expiry: "provider"`. Unblocks the `oauth` row above.
2. **`authorize` browser flow** — the loopback listener, PKCE, one-shot callback
   and grant custody described above. Depends on (1) being defined but not
   built.
3. **Per-provider OAuth endpoint configuration** — client id, authorization
   endpoint, token endpoint and default scopes as declared configuration, so no
   endpoint is frozen into code.
4. **OS keyring custody** — hold the credential in the platform keyring
   (Secret Service, Keychain, DPAPI/Credential Manager) with the file store as
   the documented fallback. The current store is a mode-`0600` file under a
   mode-`0700` root, which is the same protection openProfiler already gives an
   active provider credential; the keyring is an improvement on it, not a
   correction of it.
5. **Audit retention and rotation** — `broker-audit.jsonl` grows without bound.
   Rotation must preserve the append-only property.
6. **Provider-side revocation** — `revoke` today destroys local custody, which
   stops future mints. It does not tell the provider to invalidate an API key
   that may already have been copied. A `--at-provider` flag would, per provider.
