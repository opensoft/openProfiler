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

Profile Switcher reads profile metadata and checks credential file metadata
during discovery. Activation streams the selected credential directly into the
provider's local active home. Credential contents must never be returned to the
webview, logged, displayed, transmitted, committed, or exported.

Active credential writes must remain atomic and mode `0600` on Unix. Profile
roots and activation targets must not cross configured directory boundaries or
follow attacker-controlled symlinks.
