# Security

## Supported versions

Security fixes are applied to the latest release and the default branch.

## Reporting

Do not open a public issue for a vulnerability involving credential handling.
Report it privately through GitHub Security Advisories for this repository.

Never attach Codex `auth.json`, Claude `.credentials.json`, OAuth tokens,
browser cookies, API keys, credential logs, or private profile manifests to a
report.

## Credential boundary

openProfiler reads profile metadata and validates credential shape and
non-secret account identity during desktop eligibility checks. Activation
streams credentials directly between the selected profile and provider home.
Credential contents must never be returned to the webview, logged, displayed,
transmitted, committed, or exported.

Active credential writes must remain atomic and use mode `0600` on Unix. Profile
roots and activation targets must not cross configured directory boundaries or
follow attacker-controlled symlinks.

Windows GPT app activation must stop the packaged app before touching its
credential, reject keyring-authoritative or incomplete credentials, preserve the
destination ACL, and verify the installed account identity. The outgoing
refreshed credential is synchronized only to profiles with the same account ID.
One rollback credential may remain in the desktop Codex home until the user
confirms or undoes the switch; it receives the same protection as `auth.json`.
