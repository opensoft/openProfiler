<!--
Sync Impact Report
- Version change: template -> 1.0.0
- Added principles:
  - I. Local Credential Custody
  - II. Secret Non-Disclosure
  - III. Fail Closed and Mutate Safely
  - IV. Contract-First, Test-First Delivery
  - V. Cross-Platform Release Discipline
- Added sections: Security and Runtime Constraints; Development Workflow
- Removed sections: none (template placeholders replaced)
- Templates reviewed:
  - ✅ .specify/templates/plan-template.md (Constitution Check is feature-filled)
  - ✅ .specify/templates/spec-template.md (security requirements fit existing sections)
  - ✅ .specify/templates/tasks-template.md (test-first and release-gate tasks supported)
- Runtime guidance reviewed:
  - ✅ README.md
  - ✅ SECURITY.md
  - ✅ CONTRIBUTING.md
- Deferred items: none
-->
# openProfiler Constitution

## Core Principles

### I. Local Credential Custody

Credential material MUST remain under the current user's local control.
openProfiler MUST NOT create an undeclared second credential database, export a
credential, or move credential authority into a webview, consumer repository,
or remote service. Features that retain credentials MUST declare the exact
custody backend, reference format, lifecycle, and rollback behavior before
implementation. Consumers receive non-secret metadata or opaque references,
never reusable credential material.

### II. Secret Non-Disclosure

Credentials, access tokens, refresh tokens, authorization codes, cookies, and
provider bodies MUST NOT appear in UI payloads, logs, diagnostics, errors,
process arguments, environment variables, committed fixtures, snapshots, or
unrelated later responses. Any boundary that briefly handles a secret MUST be
bounded, independently tested with sentinels, and designed so the secret has no
owning state beyond the operation that needs it. Redaction MUST use fixed
allowlisted output rather than attempting to scrub arbitrary provider detail.

### III. Fail Closed and Mutate Safely

Missing configuration, malformed metadata, unsafe paths, symlinks, partial
credentials, unsupported credential stores, unknown protocol members, and
verification failures MUST refuse without weakening the existing posture.
Credential writes MUST be atomic and preserve the platform's required
permissions or ACLs. Rollback MUST be available whenever an operation replaces
an active reusable credential. Network and filesystem reach MUST remain
purpose-built and narrower than the feature that consumes it.

### IV. Contract-First, Test-First Delivery

Credential, automation, provider, and cross-process boundaries MUST have a
versioned governed contract before implementation. Tests MUST be authored or
updated before the production behavior they govern, demonstrate the expected
failure first when practical, and include contract, negative, redaction, and
integration cases. A checklist claim or local implementation is not release
evidence; requirements become complete only when exact tests and read-backs
prove them.

### V. Cross-Platform Release Discipline

Changes MUST preserve supported Linux, macOS, Windows, and WSL behavior unless
the governing specification explicitly narrows support. Platform-specific code
MUST remain isolated behind explicit platform gates. Every pull request MUST
pass frontend tests and build, Rust formatting, core tests, and clippy with
warnings denied. Packaging inputs MUST continue to produce the supported
Windows installer artifacts when they are affected.

## Security and Runtime Constraints

- The Tauri webview receives only purpose-built non-secret commands and data;
  generic shell, filesystem, network, dialog, and updater permissions are
  forbidden.
- Profile roots, active homes, credential targets, rollback targets, and vault
  paths MUST remain inside their declared roots and MUST reject unsafe symlink
  traversal.
- Automation input and output MUST be size-bounded and schema-closed.
- Real credentials and live provider calls MUST NOT be required for unit,
  contract, conformance, or pull-request testing.
- Committed files MUST use repository-relative or runtime-resolved paths; host-
  absolute developer paths are forbidden.

## Development Workflow

1. Govern architectural, credential, protocol, and workflow changes through
   OpenSpec before implementation.
2. Hand each governed implementation to exactly one Speckit feature and keep
   executable tasks there rather than duplicating task lists.
3. Work from a dedicated clean feature worktree; preserve unrelated dirty
   checkouts and user-authored files.
4. Run tests and builds in the declared development container when one exists;
   otherwise use the repository's documented toolchain after confirming the
   execution environment.
5. Before PR creation, run `pnpm test`, `pnpm build`, `cargo fmt --all --check`,
   `cargo test -p opensoft-open-profiler-core`, and
   `cargo clippy -p opensoft-open-profiler-core --all-targets -- -D warnings`,
   plus feature-specific contract and integration suites.
6. Security-sensitive changes require an explicit review of custody, redaction,
   path confinement, process boundaries, and rollback behavior.

## Governance

This constitution is the highest repository-local development policy. OpenSpec
proposals, Speckit artifacts, implementation plans, and pull-request reviews
MUST include a constitution check and MUST not waive a principle implicitly.

Amendments require a governed change that states the reason, affected
principles, migration impact, and verification plan. Versioning follows
semantic versioning: MAJOR for incompatible principle removal or redefinition,
MINOR for a new principle or materially expanded obligation, and PATCH for
non-semantic clarification. Every PR review MUST verify compliance; any
exception must be explicit, temporary, bounded, and approved in the governing
change.

**Version**: 1.0.0 | **Ratified**: 2026-08-25 | **Last Amended**: 2026-08-25
