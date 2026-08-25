## 0. Governance

- [x] 0.1 Record Brett's 2026-08-25 commission to implement the reviewed
  broker-revival plan, including an openProfiler-owned CLI/token contract,
  provider-native short-lived tokens, and broker-owned OAuth enrollment.
- [ ] 0.2 Create exactly one Speckit feature for implementation and record its
  branch/worktree and artifact path in this change.

## 1. Implementation handoff

- [ ] 1.1 Through the Speckit feature, publish the closed v1 schemas, positive
  and negative examples, and protocol documentation for
  `openprofiler-broker stdio`.
- [ ] 1.2 Through the Speckit feature, add the fail-closed production command
  and a separately identified deterministic conformance fixture with bounded,
  redacted I/O.
- [ ] 1.3 Through the Speckit feature, add automated tests for request bounds,
  API-key custody projection, raw-OAuth refusal, minted-capability validation,
  endpoint safety, fixed failures, and secret non-disclosure.

## 2. Consumer and realization evidence

- [ ] 2.1 Run the openxFactory consumer against the conformance fixture and
  record evidence that its binding/mint adapter accepts the official v1 shape
  and rejects malformed, unsafe, expired, or broker-issued capabilities.
- [ ] 2.2 Update openProfiler's README and security boundary, run the repository
  release checks, and record the exact green commands and implementation commit.
- [ ] 2.3 Archive only after the implementation PR is merged; an authored
  contract or local conformance run is not release evidence.
