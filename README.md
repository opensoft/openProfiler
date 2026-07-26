# openProfiler

A full LLM profile manager. openProfiler is a local-first desktop application
for discovering and activating isolated LLM profiles owned by the current user.
Its current provider integrations support Codex and Claude.

openProfiler does not create accounts or store a second credential
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
- On Windows, switch a reusable file-based ChatGPT login in the combined GPT
  desktop app after closing it, then relaunch it with rollback available.
- Record only non-secret active-profile metadata in
  `.openprofiler-active.json`.

Existing Codex or Claude processes may cache their login. Close and reopen the
provider app after CLI activation. openProfiler can manage the combined Windows
GPT/Codex desktop app's supported file credential, but it does not switch
browser sessions or Claude consumer-app session stores.

## Windows GPT app

Run the native Windows build of openProfiler to enable **Use in GPT app**.
When the combined ChatGPT/Codex desktop app uses a complete file-based ChatGPT
credential at `%USERPROFILE%\.codex\auth.json`, openProfiler:

1. asks every packaged ChatGPT/Codex window to close and waits for its processes
   to stop;
2. saves the desktop app's latest refreshed credential back into every matching
   local profile;
3. creates a restricted rollback copy;
4. atomically installs the selected profile while preserving the destination
   Windows ACL;
5. verifies the selected non-secret account identity and relaunches the app.

After checking the account and workspace in the GPT app profile menu, select
**Keep this login** to remove the rollback copy or **Undo switch** to restore the
previous login.

Only complete reusable ChatGPT OAuth bundles are eligible. An access token by
itself, an API-key profile, and a partial credential are rejected. If
`cli_auth_credentials_store = "keyring"` is configured, or file-based storage
cannot be established, openProfiler refuses token injection and leaves the
supported logout/sign-in flow in control. See the official
[Codex authentication guide](https://learn.chatgpt.com/docs/auth) for the
supported credential-store modes.

On Windows, when `%USERPROFILE%\.chatgpt-profiles` is absent and exactly one WSL
profile store exists, openProfiler discovers it through `\\wsl.localhost`.
Set the profile-store environment variables below when more than one WSL store
exists.

## Discovery

The default stores work on Linux, macOS, and Windows because they are resolved
relative to the current user's home directory.

| Provider | Manifest                                     | Profile store         | Active home |
| -------- | -------------------------------------------- | --------------------- | ----------- |
| Codex    | `~/.config/workbenches/openai-profiles.json` | `~/.chatgpt-profiles` | `~/.codex`  |
| Claude   | `~/.config/workbenches/claude-profiles.json` | `~/.claude-profiles`  | `~/.claude` |

Environment overrides:

| Purpose       | Codex                                                    | Claude                            |
| ------------- | -------------------------------------------------------- | --------------------------------- |
| Manifest      | `CODEX_PROFILES_MANIFEST` or `CHATGPT_PROFILES_MANIFEST` | `CLAUDE_PROFILES_MANIFEST`        |
| Profile store | `CODEX_PROFILES_HOME` or `CHATGPT_PROFILES_HOME`         | `CLAUDE_PROFILES_HOME`            |
| Active home   | `OPENPROFILER_CODEX_ACTIVE_HOME`                         | `OPENPROFILER_CLAUDE_ACTIVE_HOME` |

Windows desktop overrides:

| Purpose             | Variable                            |
| ------------------- | ----------------------------------- |
| Desktop Codex home  | `OPENPROFILER_CODEX_DESKTOP_HOME`   |
| Packaged GPT app ID | `OPENPROFILER_CODEX_DESKTOP_APP_ID` |

Manifest metadata takes precedence, followed by `.profile.json`, then a
credential-bearing profile directory. A malformed provider manifest is reported
without hiding valid profiles from the other provider.

The deprecated `PROFILE_SWITCHER_CODEX_ACTIVE_HOME` and
`PROFILE_SWITCHER_CLAUDE_ACTIVE_HOME` aliases remain available for compatibility.
openProfiler also recognizes the legacy `.profile-switcher-active.json` marker,
but all new activations write `.openprofiler-active.json`.

## Security model

- Profile metadata and credentials remain on the local device.
- Credential contents are never returned to the webview, logged, displayed, or
  sent over the network.
- Activation copies the selected credential through a mode-`0600` temporary file
  and atomically replaces the provider's active credential.
- Windows desktop replacement preserves the destination ACL and retains one
  restricted rollback copy until the user keeps or undoes the switch.
- Profile paths must stay beneath their configured profile root.
- Symlinked active homes and credential targets are rejected.
- The Tauri webview receives only purpose-built inventory, activation, desktop
  status, confirmation, and rollback commands.
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
cargo test -p opensoft-open-profiler-core
cargo clippy -p opensoft-open-profiler-core --all-targets -- -D warnings
pnpm tauri dev
```

The repository includes a Tauri shell, React frontend, isolated Rust core,
cross-platform icons, CI, Dependabot, security policy, and contribution guide.

## Windows installer

Version tags publish Windows installers through the
[`Windows Release`](.github/workflows/windows-release.yml) GitHub Actions
workflow. The tag must match the version in `src-tauri/tauri.conf.json`; for
example, version `0.1.0` is released with tag `v0.1.0`.

The tagged GitHub prerelease contains:

- an NSIS `-setup.exe` installer for the standard per-user installation;
- an MSI installer.

Every build also retains both installers as GitHub Actions artifacts on the
exact workflow run.

The NSIS installer creates an **openProfiler** Start-menu shortcut. These
preview installers are not code-signed yet, so Windows SmartScreen may display
a warning. Code-signing material must be supplied through GitHub Secrets and
must never be committed.

Pull requests that change release inputs build the same installers so packaging
failures are caught before merge. The workflow can also be started manually
from **Actions → Windows Release**. Pull-request and manual runs retain the
installers as workflow artifacts but do not create a GitHub Release.

## Compatible metadata

openProfiler recognizes workBenches version 1 manifests:

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
