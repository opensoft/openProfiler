# Ideation

This tree holds `Status: brainstorm` documents: free-form design exploration,
not a commitment, a specification, or a roadmap. Contradiction between
brainstorm documents is expected and permitted — the same document family may
propose competing shapes for the same problem, and none of it binds
openProfiler's implementation until it is promoted through a governed change
process. See openxFactory's
[`docs/document-lifecycle.md`](https://github.com/opensoft/openxFactory/blob/main/docs/document-lifecycle.md)
for the fuller lifecycle vocabulary (`brainstorm | staged | draft | ratified |
standard | superseded | retired | record`) these documents borrow their
`Status:` header from.

## Origin

The five documents under `brainstorm/` were rescued from a stranded shared
git checkout and re-homed here from openxFactory, where they had no
natural home (openxFactory is a domain-neutral contracts repository; these
documents are openProfiler-specific product design). The rescue and re-home
were carried out on Brett Heap's explicit, in-session ruling of 2026-09-03:
"go ahead and rehome 596 into openProfiler" (openxFactory PR #596, closed
out-of-project once its content was rescued onto a durable branch). The
rescue snapshot lives on openxFactory branch
[`rescue/openprofiler-identity-persona-brainstorm`](https://github.com/opensoft/openxFactory/tree/rescue/openprofiler-identity-persona-brainstorm)
at commit `3bc26314`. The claim and register entry for this work are tracked
on openxFactory issue
[#591](https://github.com/opensoft/openxFactory/issues/591).

## Documents

- [`brainstorm/openprofiler-identity-persona-overview.md`](brainstorm/openprofiler-identity-persona-overview.md)
  — extend openProfiler from local provider-profile switching toward a typed
  account/profile and persona model integrated with Keycloak identity,
  openXWallet authority, and OpenXPKI trust without collapsing their
  responsibilities.
- [`brainstorm/openprofiler-identity-persona-account-profile.md`](brainstorm/openprofiler-identity-persona-account-profile.md)
  — treat a provider account as verified base identity and a provider
  profile as an explicit non-secret overlay whose differences can be
  inspected and reproduced.
- [`brainstorm/openprofiler-identity-persona-authority.md`](brainstorm/openprofiler-identity-persona-authority.md)
  — use Keycloak for human identity, openXWallet for scoped authority, and
  OpenXPKI for device or workload trust while openProfiler remains
  responsible for provider account and credential operations.
- [`brainstorm/openprofiler-identity-persona-persona.md`](brainstorm/openprofiler-identity-persona-persona.md)
  — represent a model profile's persona as an optional versioned behavior
  reference, keeping it distinct from human authentication and from wallet
  authority.
- [`brainstorm/openprofiler-identity-persona-synthesis-runtime.md`](brainstorm/openprofiler-identity-persona-synthesis-runtime.md)
  — a typed profile overlay, versioned persona reference, and separated
  identity stack can produce an auditable model-runtime selection without
  making any one system own every credential or authority concern.
