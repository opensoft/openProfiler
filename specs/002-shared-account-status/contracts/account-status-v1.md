# Account Status HTTP Contract v1

The desktop native process and workloads call the same HTTPS origin. Each request presents an access token for this service. Validate the configured issuer, audience, signature, lifetime, subject, and client identity as applicable. Resolve grants on the service. Company and filter parameters narrow the authenticated scope. All status responses are private and use `Cache-Control: no-store`. The same API and response semantics apply across the supported database engines.

## Operations

For the routes below, the company prefix is `/v1/companies/{company_id}`. The account suffix is `/accounts/{provider}/{account_ref}`. Identifiers are case-sensitive opaque strings validated against the registry's identifier rules. Unknown input fields are rejected.

| Method and path after company prefix | Permission        | Result                                                                                                                                                                                                                    |
| ------------------------------------ | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GET `/accounts`                      | Scoped read       | Authorized account records, including Paused and Unknown. A valid scope with no accounts yields an empty collection. Support optional provider filtering and bounded cursor pagination. Each page is a fresh scoped read. |
| GET account suffix                   | Scoped read       | One account record. Absent and inaccessible accounts return the same 404 response.                                                                                                                                        |
| PUT account suffix `/status`         | Status management | Saves an explicit desired state against the observed revision or returns the current state on an authorized replay.                                                                                                       |
| POST account suffix `/admissions`    | Account use       | Returns a fresh allow or denial for the account and operation. Resolve the effective assignment and state together from the status authority.                                                                             |

An account record contains `company_id`, `provider`, `account_ref`, `status` (`active`, `paused`, or `unknown`), `revision` (positive decimal string or null), and `display_name` when available. Revisions are decimal strings to preserve exact values in JavaScript. List returns `accounts` and `next_cursor` (nullable). Clients retain their local account-to-profile mappings.

A write body is `{"desired_status":"paused","expected_revision":"7","request_id":"<UUID>"}`. Use `expected_revision:null` for initialization. A 200 response contains `account` (current record), `outcome` (`applied`, `noop`, or `replayed`), and `accepted_revision` (the revision of the accepted original request). A replay can have a newer `account.revision`. A lost acknowledgement is retried with the same request ID and body. A new operator decision uses a new request ID and current revision.

An admission body is `{"operation_id":"<UUID>","operation_kind":"job"}`. Kinds are `activation`, `desktop_switch`, `desktop_undo`, `job`, `turn`, and `account_reselect`. A 200 reply contains `account`, echoed `operation_id` and `operation_kind`, and `decision` (`allow`, `paused`, or `unknown`). An allow applies only to the pending operation and exact requested account. Consumers call during immediate execution, reject late or mismatched replies, and never reuse a prior reply for another operation. Cancelling a pending operation discards its reply. Work already admitted may complete.

## Errors

| HTTP status | Meaning and client action                                                                                                                                                              |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 400         | `invalid_request`. Treat schema or input errors as failures and withhold managed execution.                                                                                            |
| 401         | `authentication_required`. Clear the session's protected view and reauthenticate.                                                                                                      |
| 403         | `scope_forbidden` for an invalid company scope, or `action_forbidden` for an account visible to that subject. Do not return status from a forbidden scope.                             |
| 404         | `account_not_found` for missing or out-of-scope direct account requests. Identical public response in both cases.                                                                      |
| 409         | `stale_revision` with the current account record for an authorized editor, or `request_id_conflict` for a changed body under an accepted request ID. Present review before a new edit. |
| 503         | `authority_unavailable` for unavailable current grants or authoritative status. Do not use cached results to admit work.                                                               |

Use an allowlisted error envelope with `code` and a non-secret correlation reference. Missing or malformed replies, timeout, and other unexpected statuses are failures. Distinguish an authenticated empty inventory from an unavailable service. Contract fixtures use synthetic references and machine scopes.
