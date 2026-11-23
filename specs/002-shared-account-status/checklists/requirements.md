# Specification Quality Checklist for Shared Company Account Status

**Feature** [Shared Company Account Status](../spec.md)

**Reviewed** 2026-11-22

| Review question                                                                                | Evidence in the specification                                                                                                                                           |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Can every authorized consumer inspect the actual Paused status?                                | Story 2 and SC-002 exercise the same account and revision across both desktops, Hermes, and a domainXfactory. SC-005 keeps status visible when all accounts are paused. |
| Can a stale list, previous turn, or rollback start new work on a paused account?               | FR-005 and FR-009 define admission. SC-002 exercises queued jobs and long-lived sessions. SC-007 exercises Windows undo.                                                |
| Can the same account be managed under multiple aliases without merging unrelated accounts?     | FR-001 defines the trusted identity. SC-001 checks shared aliases and an unrelated account with the same display name.                                                  |
| Are grants evaluated at the scope the factory registry actually records?                       | FR-004 and SC-003 cover factory instance, role, company, environment, and direct account lookups. Story 2 withdraws a binding before reactivation.                      |
| What happens when the first decision, a concurrent edit, or a retry races another write?       | FR-007 and SC-004 cover initialization, stale revisions, duplicate writes, and a response lost before a later reactivation.                                             |
| Can a reader distinguish an empty result, a missing decision, a bad credential, and an outage? | FR-001, FR-006, and SC-005 through SC-006 give distinct expected outcomes.                                                                                              |
| Can old data appear after a company switch?                                                    | FR-006 and SC-005 cover a delayed response from the previous company.                                                                                                   |
| Can an existing user still activate a local-only profile?                                      | FR-009 and SC-006 cover local-only profiles and missing company bindings.                                                                                               |

This checklist records a review of the written requirements. The acceptance scenarios are for the implementation and its consumer integrations.
