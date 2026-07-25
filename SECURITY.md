# Security

## Supported versions

Security fixes are applied to the latest release and the default branch.

## Reporting

Do not open a public issue for a vulnerability involving credential handling.
Report it privately through GitHub Security Advisories for this repository.

Never attach `auth.json`, OAuth tokens, browser cookies, API keys, credential
logs, or private profile manifests to a report.

## Credential boundary

Opensoft Profile Switcher may inspect file metadata to determine whether an
isolated credential cache exists. It must never read or expose credential-file
contents. Authentication remains owned by the official Codex CLI through the
workBenches `pcodex` launcher.
