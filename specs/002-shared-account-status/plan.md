# Implementation Plan for Shared Company Account Status

**Feature** `002-shared-account-status`

**Date** 2026-11-22

**Specification** [Shared Company Account Status](spec.md)

## Summary

Host an openProfiler-owned HTTPS service in the cloud. Each deployment selects one writable PostgreSQL, MariaDB, or MySQL database as its authority for company provider account status. An operator changes the status in the openProfiler desktop app. The app's native backend, Hermes, and domainXfactory workers read the same account reference, status, and revision. Before each new managed activation or work unit, the consumer obtains a fresh decision for its selected account and continues through its existing credential and readiness checks.

## Technical Context

| Area       | Implementation choice                                                                                                                                                                                                                                     |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Service    | New Rust workspace crate, `crates/open-profiler-account-status/`. Axum for HTTP. Use SQLx's PostgreSQL and MySQL/MariaDB drivers behind the service's storage interface. Pin versions compatible with the workspace Rust baseline.                        |
| Clients    | React/TypeScript and Tauri in openProfiler. Small native HTTP client in the Tauri backend. Hermes and factory adapters use their own runtime's HTTP and identity libraries. All share the same wire contract and test vectors.                            |
| State      | One row per registered company, provider, and account. Read current status and grants from the configured writer. Status, revision, audit, and write replay information commit in one transaction.                                                        |
| Identity   | One configured pilot issuer for desktop users. Use the family Keycloak issuer where approved for the pilot, with native browser login and PKCE. Authenticate workloads with their approved machine identities and map them to server-side binding scopes. |
| Deployment | One hosted non-production service for initial acceptance, with PostgreSQL as the default pilot database. Select the engine at deployment. Desktop, Hermes, and factory instances all use the same HTTPS endpoint.                                         |
| Validation | Run the same storage and API suite on disposable PostgreSQL, MariaDB, and MySQL databases. Run the full consumer scenario against the chosen hosted pilot deployment.                                                                                     |

## Project Structure

```text
openProfiler/
  crates/open-profiler-account-status/   new service, migrations, and integration tests
  crates/open-profiler-core/src/lib.rs   profile identity, activation, and rollback primitives
  src-tauri/src/account_status.rs       new native session, profile mapping, and client module
  src-tauri/src/lib.rs                  inventory, status commands, and guarded activation
  src/App.tsx                          company inventory and operator actions
  src/types.ts                         non-secret desktop response types
  .github/workflows/ci.yml              service, UI, and platform verification
  specs/002-shared-account-status/      spec, plan, contract, scenarios, and tasks
```

## Governing Boundaries

- The hosted service owns shared status and stores non-secret references. Native provider credential writes and rollback stay on the device. Hermes and factories use their existing credential stores and execution paths.
- The account catalog and effective grants come from their authorized owners. openProfiler applies those grants on every read, write, retry, and admission. The company account key is stable across desktop and workload bindings. Company selection, token claims, and request fields cannot supply authority on their own.
- The first code change establishes the versioned API, authorization tests, and transaction tests on all three supported databases. Use the same sanitized fixtures across repositories. Run openProfiler's existing frontend, Rust, and Windows gates as part of validation.
- The active checkout has no `.specify/` scaffold. This plan uses the repository's tracked contribution and credential rules and the saved Speckit layout. The saved constitution's contract-first, credential custody, and platform provisions inform the design. Use the governing OpenSpec process before implementation if the implementation baseline adopts that constitution.

## Component Changes

### openProfiler

1. Add the service crate with migrations, OIDC validation, a resolver for company account registrations and effective grants, and the status API. Query registrations and effective grants alongside status on the configured writer. Serialize edits against the registered account row, including the first decision. Publish a status change only after commit.
2. Add native desktop login and a bounded client for status, edit, and admission. Resolve each managed local profile to a registered company provider account through trusted enrollment and provider identity evidence where supported. An operator can manage an enrolled account even if the device has no usable local credential for it.
3. Add a company view in `src/App.tsx` for Active, Paused, Unknown, and lookup failures. Support explicit refresh and show the last successful check. Use one action ID per operator action. Retain it for an uncertain save retry, discard late responses from older revisions or former company sessions, and require a new user action on conflict.
4. Guard the generic activation and Windows switch and undo commands in `src-tauri/src/lib.rs`. Resolve and verify the target under the local activation guard, obtain the decision before any credential write or provider shutdown, and revalidate the target used by the mutation. Treat switch and undo as single managed operations. Preserve the existing discard-rollback command and local-only workflow. Update core rollback provenance only as needed to associate the actual backup with its managed account and safely handle existing backups.

### Hermes

- Company Hermes deployments such as `FarHeap/Hermes-Install/k8s/prod/workloads/deployment/hermes-farheap.yaml` pin a Hermes agent image and boot `hermes gateway run`. Wire non-secret endpoint, company, and account binding inputs through the chosen QA overlay and existing Hermes profile renderer. Mount or obtain the workload identity through the existing platform mechanism. Keep the checked-in production manifest as a discovery reference.
- In the pinned Hermes agent source or maintained derivative, attach the status client to the final account routing and turn entry points. Upstream anchors are `gateway/run_turn.py`, `gateway/platforms/api_server.py`, and `agent/conversation_loop.py`. Verify the equivalent paths in the deployed source, including API, message, resume, and fallback paths enabled in the pilot. Check the final account before the first new provider call in each independently dispatched turn, including turns in existing sessions. Recheck if the runtime selects another account. Expose the scoped status view to operators through the existing diagnostics or control surface.
- For deployments using `xFactory-Hermes-Install`, trace `src/hermes_install/api/routers/jobs.py` and `domain/run_loop.py` through `api/routers/runs.py` to the exact worker launch. Claim reserves work. Place admission at final dispatch and require the result for the running transition and the provider launch. If creation of a run can precede a further queue, carry the chosen account reference through it and place the fresh decision at the actual launch boundary.

### domainXfactories

- Implement the shared behavior in the installed executor. For the Omnigent path, `Omnigent-Install/hermes_service/dispatcher.py::dispatch_job` stages the inbox. Check at `collect_worker_inbox_item` before admitting immediate execution. If the claimed worker can wait before launch, move the decision to that worker's final start and link the lifecycle transition to it. Apply the same check in `hermes_service/launcher.py::launch_job` before direct launch. Verify both paths from `hermes_service/server.py`. Keep queue ownership consistent when admission is denied or unavailable. Return a typed outcome and leave the job recoverable under existing queue rules.
- Extend `OmniWorker-Install/scripts/worker_pool_binding.py` and `scripts/validate_worker_preflight.py` to resolve a trusted account reference from the deployed profile binding, retain existing per-worker provider readiness, and reject incomplete managed configuration. Repin the tested worker release through `Omnigent-Install/contracts/omniworker-install-pin.yaml` for that runtime.
- Use Medx and Ledger as representative domainXfactory pilots, subject to finding compatible non-production installations with a shared company scope. Bind A and B to one instance and B to the second through actual instance grants, role, and environment. Store instance-specific settings in the governed installation configuration. Use each domain's `stack.yaml` or its actual installation pin mechanism to consume the qualified runtime. Change reusable domain profiles or overlay manifests only if the validated schema or overlay content needs it. Carry this same conformance suite to every other runtime and domain instance when it enrolls.

### Effective Account and Grant Source

The credential registry has account and binding templates and pinned releases. Introduce a server-side projection of approved accounts and grants for the pilot. The registry or company authorization owner publishes complete, versioned scope updates through an authenticated control-plane importer. Define effective grants as the applied and acknowledged revision for participating deployments. Require the applicable revocation workflow to publish into this path before acknowledging withdrawal. Reject older or ambiguous updates and use a bounded validity policy for publisher freshness. If the pilot cannot establish that the projection reflects the current binding authority, the affected scope reports unavailable. Each decision reads the effective scope and account state from one statement snapshot on the writer. The admin publisher and consumer interfaces have distinct permissions. There are no end-user routes for creating bindings in this feature.

## Hosted Persistence

- Select the engine explicitly as `postgres`, `mariadb`, or `mysql`. The service uses a private, TLS-verified writer connection from its deployment configuration. Keep the connection secret server-side. Detect engine or schema mismatch at startup and expose readiness only when the selected engine is usable.
- Keep account status rules in `crates/open-profiler-account-status/src/store.rs` and isolate driver selection, normalized SQL binds, and database error mapping in the SQLx store module. Qualify MariaDB and MySQL separately even where they share SQL. Use dialect migrations under `migrations/`.
- Run critical reads on the writer, with status and effective grants read together. Use explicit transaction isolation, stable row locks, and deterministic lock order for writes and grant publication. Verify commit durability and failover on the hosted database before enrollment. If the service cannot establish that the writer contains acknowledged changes, return unavailable for affected scopes until reconciled.
- Run the storage and HTTP suites on PostgreSQL, MariaDB, and MySQL before declaring the feature supported. Proposed first CI baselines are PostgreSQL 17, MariaDB 11.4 LTS, and MySQL 8.4 LTS. Pin maintained releases and qualify the exact hosted engine and topology used for deployment.

## Delivery Sequence

1. Agree the stable account keys, approved issuer and machine subjects, publisher ownership, runtime commit/image pins, and protected environment. Validate the shared contract with fixtures before changing consumer code.
2. Implement the service and openProfiler operator workflow. Run the two-desktop acceptance scenario against one database before integrating machines. The operator workflow is the first demonstrable increment.
3. Integrate Hermes and the shared factory runtime. Integrate Medx and Ledger through their qualified installation profiles, prove exact dispatch boundaries, and run the combined scenario described in the [QA quickstart](quickstart.md).
4. Verify concurrent edits, lost acknowledgement, old responses, outage, grant changes, and Windows undo. Roll out by participating workload scope after its read and admission paths pass. Introduce managed bindings only after all enabled entry points use the contract. For a rollback, suspend managed dispatch or return to a known enforcing build, keep existing work recoverable, and retain the database and audit history. Resume with the same authority and current revisions.

## Design Artifacts

- [Research and decisions](research.md)
- [Data model](data-model.md)
- [Shared HTTP contract](contracts/account-status-v1.md)
- [QA quickstart](quickstart.md)
- [Implementation tasks](tasks.md)
