# Profile Switcher

A local-first desktop application for discovering and activating isolated Codex
and Claude profiles owned by the current user.

Profile Switcher does not create accounts or store a second credential
database. It relies on profile directories created by
[workBenches](https://github.com/opensoft/workBenches) or another compatible
setup, then makes one of those local credentials active in the provider's
standard user home.

## Current scope

- Discover Codex profiles under `~/.chatgpt-profiles/profiles`.
- Discover Claude profiles under `~/.claude-profiles/profiles`.
- Read optional workBenches manifests and `.profile.json` metadata.
- Find credential-bearing profile directories even when no manifest exists.
- Show provider, profile name, declared identity, family, aliases, source, and
  readiness.
- Activate Codex by atomically replacing `~/.codex/auth.json`.
- Activate Claude by atomically replacing `~/.claude/.credentials.json`.
- Record only non-secret active-profile metadata in
  `.profile-switcher-active.json`.

Existing Codex or Claude processes may cache their login. Close and reopen the
provider app after activation. This project switches local Codex and Claude Code
credentials; it does not switch browser sessions or unrelated ChatGPT/Claude
consumer-app session stores.

## Discovery

The default stores work on Linux, macOS, and Windows because they are resolved
relative to the current user's home directory.

| Provider | Manifest                                     | Profile store         | Active home |
| -------- | -------------------------------------------- | --------------------- | ----------- |
| Codex    | `~/.config/workbenches/openai-profiles.json` | `~/.chatgpt-profiles` | `~/.codex`  |
| Claude   | `~/.config/workbenches/claude-profiles.json` | `~/.claude-profiles`  | `~/.claude` |

Environment overrides:

| Purpose       | Codex                                                    | Claude                                |
| ------------- | -------------------------------------------------------- | ------------------------------------- |
| Manifest      | `CODEX_PROFILES_MANIFEST` or `CHATGPT_PROFILES_MANIFEST` | `CLAUDE_PROFILES_MANIFEST`            |
| Profile store | `CODEX_PROFILES_HOME` or `CHATGPT_PROFILES_HOME`         | `CLAUDE_PROFILES_HOME`                |
| Active home   | `PROFILE_SWITCHER_CODEX_ACTIVE_HOME`                     | `PROFILE_SWITCHER_CLAUDE_ACTIVE_HOME` |

Manifest metadata takes precedence, followed by `.profile.json`, then a
credential-bearing profile directory. A malformed provider manifest is reported
without hiding valid profiles from the other provider.

## Security model

- Profile metadata and credentials remain on the local device.
- Credential contents are never returned to the webview, logged, displayed, or
  sent over the network.
- Activation copies the selected credential through a mode-`0600` temporary file
  and atomically replaces the provider's active credential.
- Profile paths must stay beneath their configured profile root.
- Symlinked active homes and credential targets are rejected.
- The Tauri webview receives only two purpose-built commands: list profiles and
  activate a selected discovered profile.
- There is no generic shell, filesystem, network, dialog, or updater permission.

Treat all provider credential files as passwords. Never commit, paste, export,
or share them.

## Development

Prerequisites:

- Rust 1.88 or newer
- Node.js 22
- pnpm 11
- Tauri 2 system dependencies for your platform

```bash
pnpm install
pnpm test
pnpm build
cargo test -p opensoft-profile-core
cargo clippy -p opensoft-profile-core --all-targets -- -D warnings
pnpm tauri dev
```

The repository includes a Tauri shell, React frontend, isolated Rust core,
cross-platform icons, CI, Dependabot, security policy, and contribution guide.

## Compatible metadata

Profile Switcher recognizes workBenches version 1 manifests:

```json
{
  "version": 1,
  "profiles": [
    {
      "name": "work",
      "profilePath": "company/work",
      "family": "company",
      "email": "developer@example.com",
      "aliases": ["office"]
    }
  ]
}
```

Only `name` is required. `profilePath` defaults to `name`; absent family and
identity metadata are displayed as local or undeclared.

## Project origin

This is an independent Apache-2.0 implementation built around public provider
configuration behavior and workBenches profile conventions. No source code,
visual assets, text, or credential-handling implementation was copied from
`Lampese/codex-switcher`, whose repository did not declare a license when this
project was started.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
