# Feature Specification for Shared Company Account Status

**Feature** `002-shared-account-status`

**Created** 2026-11-22

**Status** Draft

An authorized operator can pause or reactivate a company provider account once in openProfiler. All authorized openProfiler apps, Hermes instances, and domainXfactories can read the same saved status for that account. Each consumer sees only the accounts within its scope. Several accounts can be Active at the same time.

## User Scenarios & Testing

### User Story 1 - Change a company account once (Priority P1)

As a company profile operator, I can see which of our accounts are Active or Paused and change their status from openProfiler.

**Independent test**

Give the operator ten enrolled Claude accounts. Open the inventory on two devices and select `claude-03` on the second device.

1. Pause `claude-03` on the first device. After the save is acknowledged, refresh the second device. Both show Paused for the same account and the other nine retain their prior statuses.
2. The second device shows both Paused and Selected on this device. The existing sign-in can continue. Attempting a new managed activation through either of two profiles bound to `claude-03` is refused.
3. Reactivate `claude-03`. The second device shows Active on refresh and can select either profile if local credential checks pass.

### User Story 2 - Read account status and select an account for new work (Priority P1)

As a Hermes or domainXfactory operator, I can inspect the status of each account assigned to my workload and tell which accounts are available for its next job.

**Independent test**

Give Hermes and one factory instance valid bindings to accounts A and B. Give a second factory instance access only to B. Use the factory instances' actual identities, roles, and environments. Start work on A and queue another job that has seen A in its candidate list.

1. Pause A from openProfiler. On their next reads, both desktops, Hermes, and the first factory can inspect A as Paused at the same revision. Hermes and the first factory can still select B. The second factory sees only B.
2. Let the admitted work finish. Dispatch the queued job after the pause is acknowledged. Its old candidate list cannot authorize A. Also try starting a new turn on A in a long-lived Hermes session.
3. Reactivate A. The readers with access see the new Active revision. A is selectable again wherever an effective binding and a usable runtime profile exist. Repeat with the first factory's binding to A withdrawn before reactivation. That factory now sees only B.

### User Story 3 - Resolve concurrent changes and uncertain saves (Priority P2)

As an operator, I can tell whether my action took effect when an acknowledgement is lost or another operator edits the account.

**Independent test**

1. Two operators read A as Active at revision 1. One pauses A at revision 2 and reactivates it at revision 3. The other submits a pause based on revision 1. The stale edit is refused and shows Active at revision 3.
2. Repeat with a lost acknowledgement for the pause. Retry that request both before and after reactivation. In both cases the account history contains exactly one pause for the original action. After reactivation the response identifies revision 3 as current.
3. Deliver the revision 2 status response after the desktop has displayed revision 3. The desktop continues to show revision 3.

## Requirements

### Functional Requirements

- **FR-001 — Account identity and first status.** Use a stable company, provider, and account reference. Every managed profile for that account inherits one shared status. An operator authorized to manage that account can set the first status of a registered account to Active or Paused. Until that decision, authorized readers see Unknown and consumers exclude the account from new work. A profile requires a trusted association with its account. An email, alias, matching name, locally selected login, or registry lifecycle label is insufficient.
- **FR-002 — Operator actions.** Show authorized operators the accounts they may manage, including paused accounts. Permit them to set an explicit desired status on one account at a time. Only an authorized status manager for that account can initialize or change it. Pausing and reactivating do not change account assignments.
- **FR-003 — Shared reads.** Return the account reference, shared status, and per-account revision to every authorized reader, including headless consumers. An account awaiting its first decision reports Unknown with no saved revision. After a save is acknowledged, a fresh read must return that revision or a later committed revision. Reads that overlap a transition may observe either side. A desktop must refresh on opening the view or on user refresh, show when it last checked, and discard older responses for the same account. Display local selection and credential readiness separately.
- **FR-004 — Access scope.** Evaluate the authenticated subject's current grants for lists, individual lookups, admission, and writes, including retries. Honor the applicable client, environment, host, persona, allowed factory instance, and role in a workload binding. Caller-supplied filters may only narrow the result. An out-of-scope account must be absent from lists and indistinguishable from a nonexistent account on direct lookup. Read permission alone grants no authority to change status.
- **FR-005 — Admission to new work.** At the point of selecting a company account for a managed activation, a queued job leaving the queue, a new independently dispatched turn, or a retry that selects another account, obtain a fresh decision for that specific operation and account. Admission requires a current assignment and Active status. Consumers also perform their existing credential checks. A list response, an earlier job's admission, or an open provider session cannot admit new work. A successful decision admits that one operation. A subsequent pause allows the admitted operation to finish. A decision begun after an acknowledged pause must deny the account while it is paused.
- **FR-006 — Loss of access or connectivity.** If current status or assignment cannot be verified, report the lookup failure and withhold admission. A desktop may show a dated, stale status only while it can establish the viewer's right to that account. On sign-out, company switch, or loss of access, remove the prior scope's protected data from view and discard outstanding responses for it. Distinguish an unavailable lookup from a successful lookup with zero selectable accounts.
- **FR-007 — Writes, revisions, and audit.** Confirm a save only once its status, revision, and audit entry are durable together. Record the company, provider, account, actor, time, and old and new states, including the initial decision. Order transitions by a strictly increasing per-account revision. A new edit must identify the revision the operator saw (or the absence of a first decision). A request based on the current revision that asks for the already saved state succeeds as a no-op. A stale edit cannot overwrite a newer revision, even when the value has cycled back. A replay is attributable to the same operator, account, and original request. Replaying a successful request must report the current state and revision to a still-authorized operator without repeating the transition.
- **FR-008 — Consumer contract.** Authorized inventory and detail reads include assigned Active, Paused, and as-yet-undecided accounts. Selection for use considers only assigned Active accounts. Both use the same authority for status. Use non-secret references for accounts and profiles. Provider credentials and session material must stay out of shared status, responses, logs, and history.
- **FR-009 — Desktop compatibility.** Apply the admission rule on every managed activation path, including a desktop switch or undo that would install a managed login for active use. A denial must occur before stopping the provider app or replacing its credential. Preserve the ability to discard a rollback copy. A known company-managed profile with a missing or ambiguous account binding is unavailable for activation. Existing local profiles outside company management retain their local workflow without enrollment. A missing or broken association on a previously managed profile cannot reclassify it as local-only.

### Key Entities

- **Company provider account** — The object an operator pauses or reactivates. Multiple aliases and profiles can refer to it.
- **Account binding** — An existing grant connecting a consumer to an account under a specific scope. Different consumers may see different subsets of the same company's accounts.
- **Status revision** — The version of one account's saved decision. Revisions are comparable for the same account.
- **Admission** — A consumer's decision to start one managed operation using a particular account.

## Success Criteria

| ID     | Acceptance condition                                                                                                                                                                                                                                                                                                                          |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SC-001 | The ten-account, two-desktop test in Story 1 passes for pause and reactivation. A second alias observes the same decision. An identically named profile linked to a different account keeps its own status.                                                                                                                                   |
| SC-002 | In Story 2, every reader assigned A can explicitly read A's Paused and Active revisions. A job queued before the pause and a new turn in an existing session both decline A after the pause. The previously admitted job completes.                                                                                                           |
| SC-003 | An unassigned factory, a caller in the wrong company or environment, and a caller using an ungranted factory role cannot list or discover A by ID. A permitted read-only caller sees A's status and cannot change it. Revoking a binding takes effect on the next lookup and admission.                                                       |
| SC-004 | The stale edit, delayed response, and retries in Story 3 produce exactly the two intended transitions after initialization. A repeated request for the current state leaves its revision unchanged. Two competing first decisions can initialize an account only once.                                                                        |
| SC-005 | With all assigned accounts paused, each workload can still read their statuses and reports zero selectable accounts. With the status authority unreachable, it reports a lookup failure and admits zero new jobs on cached data. Switching companies with an old response in flight shows no data from the previous company.                  |
| SC-006 | A registered account with no decision reports Unknown until an operator sets its first status. A missing managed binding cannot become a local-only activation. An existing local profile outside company management can still be selected during an outage. An Active account with an unusable credential fails the runtime readiness check. |
| SC-007 | On a supported Windows desktop, pause the managed target of a pending undo. An attempted undo is refused before the running app stops or the active credential changes. After reactivation, undo follows the existing credential and identity checks.                                                                                         |

## Assumptions and Scope

openProfiler owns the shared status. Existing company and workload bindings determine who can read or use an account. The first acceptance case uses Claude subscriptions. A registered account may be visible to its manager before a usable profile is present on a particular device.

This feature governs account use initiated through participating openProfiler, Hermes, and domainXfactory clients. Quota monitoring, automatic rotation, OmniRoute fallback, credential distribution, and provider-wide sign-out are outside its scope.

## Related Documents

- [openProfiler account and profile model](../../docs/ideation/brainstorm/openprofiler-identity-persona-account-profile.md)
- [openProfiler desktop behavior](../../README.md#windows-gpt-app)
- [openProfiler credential boundary](../../SECURITY.md#credential-boundary)
- [Company credential broker proposal](https://github.com/opensoft/openProfiler/pull/20/files)
- [xFactory credential binding template](https://github.com/opensoft/xFactory-Agents-Credential-Registry/blob/c004d69/templates/credential-binding.yaml)
- [xFactory provider account template](https://github.com/opensoft/xFactory-Agents-Credential-Registry/blob/c004d69/templates/provider-account.yaml)

## QA binding renewal

The hosted service can renew the approved QA binding projection every six
hours while the current, unexpired members, accounts, and grants match the
deployment policy. Renewal advances the projection generation and extends its
lease without changing account status revisions. A missing, expired, or
different projection requires an authorized publisher to restore it. The
renewal task cannot create or replace grants.
