# Feature Specification: Model Provider Broker CLI Contract

**Feature Branch**: `001-broker-cli-contract`

**Created**: 2026-08-25

**Status**: Approved for implementation

**Input**: User description: "Implement the governed openProfiler model-provider broker stdio contract, fail-closed command, schemas, conformance fixture, security documentation, and tests."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Store an API key without retaining it in the consumer (Priority: P1)

A local model-consuming application gives an API key to openProfiler once and
receives an opaque credential reference that it can safely persist. The key is
never returned, logged, or copied into the consumer's repository or state.

**Why this priority**: Long-lived credential custody is the security boundary
that makes every later broker operation possible.

**Independent Test**: Submit a synthetic API-key custody request to the
conformance surface and verify that only an opaque reference is returned and
that the supplied sentinel appears nowhere else.

**Acceptance Scenarios**:

1. **Given** a valid local API-key custody request, **When** the broker accepts
   the key, **Then** the caller receives a non-secret opaque reference and no
   response or diagnostic contains the key.
2. **Given** raw OAuth material, **When** it is submitted through the custody
   command, **Then** the broker refuses it and directs enrollment to the
   broker-owned interactive flow without retaining or echoing the value.

---

### User Story 2 - Mint a short-lived provider capability (Priority: P1)

A local model-consuming application presents a credential reference and
receives a short-lived capability it can use directly with the declared model
provider. The application can verify the capability's token type, expiry,
scope, audit facts, protocol, and safe routing metadata before use.

**Why this priority**: The broker must leave the latency-sensitive provider
request path while still constraining what the consumer is allowed to use.

**Independent Test**: Mint from a synthetic reference and verify the complete
capability shape, then test that expired, overlong, unscoped, unsafe, or
broker-issued capabilities are refused.

**Acceptance Scenarios**:

1. **Given** a valid credential reference, **When** the broker mints a
   capability, **Then** the caller receives a provider-native bearer token with
   a future expiry of at most one hour, explicit provider-native scopes,
   auditable grant metadata, and safe provider routing metadata.
2. **Given** an expired token, unknown protocol, empty scope, unsafe endpoint,
   or wrong token type, **When** the capability is validated, **Then** no token
   is released to the caller.

---

### User Story 3 - Integrate safely before a production vault exists (Priority: P2)

A consumer developer can validate integration against deterministic synthetic
examples while production openProfiler remains honestly unavailable until a
credential backend is installed.

**Why this priority**: It lets consumers implement and test the boundary
without encouraging a temporary plaintext vault or requiring paid provider
calls.

**Independent Test**: Run all positive and negative examples through the
published conformance surface without network access, then invoke the
production command without a backend and verify the fixed unconfigured refusal.

**Acceptance Scenarios**:

1. **Given** no production custody backend, **When** the production command is
   invoked, **Then** it returns a fixed unconfigured refusal and stores or
   mints nothing.
2. **Given** the conformance surface, **When** a consumer executes the
   published synthetic cases, **Then** every response is deterministic,
   contract-valid, offline, and clearly identified as test-only.

### Edge Cases

- Input is empty, malformed, non-UTF-8, larger than 262,144 bytes, or contains
  another JSON value after the first object.
- A request contains unknown, missing, duplicated, or wrong-typed fields.
- A binding identifier, reference, scope, or audit value is blank or exceeds
  its declared bound.
- An OAuth request includes an authorization code, access token, refresh token,
  or other raw credential material.
- A minted capability expires exactly now, exceeds one hour, carries duplicate
  scopes, or names an unknown token or provider protocol.
- An endpoint includes URL user information, is relative, uses an unsafe HTTP
  host, or exceeds its bound.
- A backend error contains credential, token, filesystem, command, or provider
  detail that must be redacted.
- Standard output or standard error would exceed the response bound.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The product MUST expose one official non-interactive broker
  command that accepts exactly one bounded request from standard input and
  writes exactly one bounded response before exiting.
- **FR-002**: Every request and response MUST carry contract version 1 and one
  supported operation, and unknown fields or operations MUST be refused.
- **FR-003**: Credentials and tokens MUST NOT be accepted through process
  arguments or environment variables.
- **FR-004**: API-key custody MUST return only an opaque credential reference;
  the supplied value MUST NOT appear in responses, diagnostics, logs, profile
  data, repository data, or later broker operations.
- **FR-005**: Raw OAuth material MUST be refused by the non-interactive custody
  operation; OAuth enrollment MUST remain on a broker-owned interactive
  surface and expose only a resulting reference to consumers.
- **FR-006**: Minting MUST accept an opaque reference and return only a
  provider-native bearer capability.
- **FR-007**: Every minted capability MUST expire in the future within one hour
  and MUST declare at least one unique opaque provider-native scope.
- **FR-008**: Every minted capability MUST declare who issued it, who approved
  it, and the audit reference for the grant.
- **FR-009**: Every minted capability MUST identify the supported provider
  protocol and provide absolute catalog and completion endpoints plus a bounded
  request timeout.
- **FR-010**: Provider endpoints MUST use HTTPS, except HTTP MAY be accepted for
  literal loopback hosts; URL user information MUST be refused.
- **FR-011**: A minted token MUST remain memory-only, MUST be discarded at
  expiry, and MUST NOT trigger automatic re-mint or retry of an expired call.
- **FR-012**: Failures MUST use a closed set of fixed machine-readable codes,
  exit non-zero, and disclose no request, credential, token, path, command,
  provider response, or backend detail.
- **FR-013**: The product MUST publish schemas and positive and negative
  examples for every v1 operation and response family.
- **FR-014**: The product MUST provide a deterministic, test-only conformance
  surface that requires no real credential, filesystem vault, provider account,
  or network call.
- **FR-015**: The production command MUST return a fixed unconfigured refusal
  and perform no custody or minting when no production backend is installed.
- **FR-016**: Existing desktop profile discovery and activation behavior MUST
  remain unchanged.

### Key Entities

- **Broker Request**: A versioned, closed request naming one operation and the
  bounded fields that operation requires.
- **Credential Reference**: An opaque non-secret identifier for credential
  material held by a broker-owned backend.
- **Minted Capability**: A provider-native bearer token plus expiry, scopes,
  grant audit facts, and routing metadata; it exists only in memory.
- **Provider Routing Metadata**: The closed provider protocol, safe endpoints,
  and timeout a consumer validates before attaching authorization.
- **Failure Envelope**: A closed, fixed, redacted machine-readable refusal.
- **Conformance Case**: A synthetic request and expected response or refusal
  that proves protocol behavior without production dependencies.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: One hundred percent of published positive and negative examples
  pass their schemas and produce the documented success or refusal.
- **SC-002**: Sentinel API keys and tokens appear zero times in repository
  files, persisted test state, diagnostics, failure responses, and unrelated
  later responses after each conformance run.
- **SC-003**: Every malformed, expired, overlong, unscoped, wrong-type,
  unknown-protocol, or unsafe-endpoint capability is refused before a consumer
  can use its token.
- **SC-004**: The complete conformance suite runs offline and completes in
  under 30 seconds on the supported development environment.
- **SC-005**: Invoking production without a backend always produces the same
  fixed unconfigured refusal and creates zero credential or token files.
- **SC-006**: All existing profile discovery, activation, frontend, and release
  checks continue to pass unchanged.

## Assumptions

- The first consumer is openxFactory doxBench, which invokes a declared argv
  without a shell and keeps minted capability in server memory.
- Provider-native scopes are opaque strings; neither openProfiler nor the
  consumer infers authority from their spelling.
- The initial production command is permitted to be fail-closed until a
  separately governed credential backend is available.
- Provider-specific OAuth enrollment, refresh, revocation, and vault storage
  are separate features that must conform to this contract.
- The supported model provider implements the declared xFactory workbench
  protocol; normalizing unrelated provider APIs is outside this feature.
