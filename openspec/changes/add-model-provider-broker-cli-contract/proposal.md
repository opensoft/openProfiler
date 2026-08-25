## Why

openxFactory's doxBench model-provider boundary is ready to consume a local
credential broker, but openProfiler does not yet declare a non-interactive
command or wire contract for credential custody and short-lived token minting.
Without an authoritative surface in the repository that owns the credentials,
consumers either invent incompatible commands or freeze an accidental adapter
prototype as the contract.

## What Changes

- Declare the `openprofiler-broker stdio` command as a bounded, one-request
  JSON-on-standard-input / one-response JSON-on-standard-output protocol.
- Declare `store_credential` for API-key custody and `mint` for issuance from
  an opaque credential reference. OAuth enrollment remains broker-owned and
  supplies a reference to consumers; an OAuth authorization code or refresh
  token never crosses this CLI surface.
- Declare minted capabilities as provider-native bearer tokens with an
  absolute expiry, explicit provider-native scopes, and bounded provider
  routing metadata for the xFactory workbench provider protocol.
- Require fixed machine-readable failures on standard output, non-zero exit on
  failure, bounded input/output, and no secret material on standard error.
- Add schemas, examples, and a conformance fixture so consumers can test the
  protocol without a real provider credential or network call.
- Keep credential-vault storage and provider-specific OAuth enrollment out of
  this change; those implementations must satisfy this contract when added.

## Capabilities

### New Capabilities

- `model-provider-broker-cli`: The official openProfiler stdio command,
  request/response shapes, token semantics, security rules, and conformance
  behavior for model-provider consumers.

### Modified Capabilities

None.

## Impact

- Adds a public CLI contract owned by openProfiler and consumed first by
  openxFactory's doxBench model-provider adapter.
- Adds contract schemas/examples and a deterministic conformance executable or
  fixture; no real credential, account, provider endpoint, or network access is
  required for tests.
- Extends openProfiler's security documentation from profile activation to
  brokered credential references and memory-only minted capabilities.
- Does not change the current desktop profile-switching behavior.
