## ADDED Requirements

### Requirement: The broker exposes one closed stdio command

openProfiler SHALL expose `openprofiler-broker stdio` as the official
non-interactive broker command. The command MUST read exactly one UTF-8 JSON
object from standard input to EOF, MUST refuse input larger than 262,144 bytes,
MUST write exactly one UTF-8 JSON object to standard output, and MUST terminate
after that response. Every request and response MUST carry
`schema_version: 1` and a supported operation. Credential or token material
MUST NOT be accepted through argv or environment variables.

#### Scenario: One valid request completes

- **WHEN** a caller invokes `openprofiler-broker stdio` with one conforming v1
  request on standard input
- **THEN** the command writes one conforming JSON response, exits, and emits no
  secret material on standard error

#### Scenario: Oversized or trailing input is refused

- **WHEN** standard input exceeds the byte bound or contains data after the
  single JSON object
- **THEN** the command exits non-zero with a fixed failure response and does not
  execute a custody or mint operation

### Requirement: API-key custody returns only an opaque reference

The `store_credential` operation SHALL accept exactly `schema_version`,
`operation`, `binding_id`, `auth_kind`, and `credential`. It MUST accept raw
credential material only for `auth_kind: "api_key"`, MUST pass the material to
the configured broker backend without logging or echoing it, and MUST return
only an opaque `credential_ref` plus the fixed success envelope. It MUST NOT
write the supplied value to a profile directory, repository, response, error,
or diagnostic.

OAuth enrollment SHALL remain on an openProfiler-owned interactive surface.
The stdio command MUST refuse raw OAuth authorization codes, access tokens, and
refresh tokens; consumers may use only a `credential_ref` produced by that
broker-owned flow.

#### Scenario: An API key is stored

- **WHEN** a conforming `store_credential` request supplies an API key
- **THEN** the backend receives the key once and the response contains an
  opaque reference but not the supplied value

#### Scenario: Raw OAuth material is refused

- **WHEN** `store_credential` names `auth_kind: "oauth"` or supplies OAuth
  material
- **THEN** the command returns the fixed `oauth_enrollment_required` refusal
  without retaining or echoing the supplied value

### Requirement: Mint returns a provider-native bounded capability

The `mint` operation SHALL accept exactly `schema_version`, `operation`,
`binding_id`, `auth_kind`, and `credential_ref`. A successful response MUST
carry `token_type: "provider_native_bearer"`, a non-empty bearer `token`, an
integer Unix `expires_at` no more than 3,600 seconds in the future, a non-empty
duplicate-free array of opaque provider-native `scope` strings, and `grant`
metadata containing non-empty `issued_by`, `approved_by`, and `audit_ref`.

The response MUST carry provider metadata containing the closed protocol
identifier `xfactory_workbench_v1`, absolute catalog and completion endpoints,
and a bounded timeout. Endpoints MUST use HTTPS except that HTTP MAY be used
with a literal loopback host. URL user information MUST be refused.

The minted token MUST remain memory-only, MUST NOT appear in logs, files,
errors, or later responses, and MUST be discarded at expiry. An expired call
MUST NOT be automatically re-minted or retried; a later independent operation
may mint again.

#### Scenario: A capability is minted

- **WHEN** a valid credential reference resolves to a provider grant
- **THEN** the command returns a provider-native bearer capability with bounded
  expiry, explicit scope, grant audit metadata, and valid routing metadata

#### Scenario: An unusable capability is refused

- **WHEN** the backend returns an expired token, an empty scope, an unknown
  protocol, or an unsafe endpoint
- **THEN** the command returns a fixed failure and no token reaches the caller

### Requirement: Failures are fixed, bounded, and redacted

Every failure SHALL exit non-zero and return a closed JSON object containing
`schema_version: 1`, `ok: false`, and one fixed error code. Error codes MUST NOT
contain request, credential, token, provider response, path, command, or backend
details. Standard error MUST be empty or a fixed non-secret diagnostic.

The v1 error set SHALL include `invalid_request`, `unsupported_operation`,
`oauth_enrollment_required`, `credential_unavailable`, `broker_unconfigured`,
`mint_failed`, and `internal_error`.

#### Scenario: A backend includes secret detail in an error

- **WHEN** a custody or mint backend fails with credential-shaped or
  provider-shaped detail
- **THEN** the command returns only the corresponding fixed v1 error code and
  no backend detail appears on either output stream

### Requirement: Conformance requires no real credential or network

openProfiler SHALL publish v1 JSON schemas, positive and negative examples, and
a deterministic conformance fixture that implements the stdio protocol using
synthetic sentinel values. The fixture MUST make no network request, MUST write
no credential store, and MUST be distinguishable from the production command.
The production command MUST fail with `broker_unconfigured` when no custody
backend is installed.

#### Scenario: A consumer runs the conformance fixture

- **WHEN** a consumer sends the published synthetic store and mint requests to
  the fixture
- **THEN** it receives schema-valid deterministic responses without a real
  account, provider credential, filesystem vault, or network call

#### Scenario: Production has no backend

- **WHEN** `openprofiler-broker stdio` runs without a configured custody backend
- **THEN** it returns `broker_unconfigured` and stores or mints nothing
