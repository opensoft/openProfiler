# Opensoft Profile Switcher

A local-first desktop manager for the isolated Codex profiles created by
[workBenches](https://github.com/opensoft/workBenches).

The application reads profile inventory metadata and checks whether a
profile-local `auth.json` exists and is non-empty. It never reads, returns,
copies, exports, or commits credential contents.

## Initial capabilities

- Discover `~/.config/workbenches/openai-profiles.json`.
- Respect `CODEX_PROFILES_MANIFEST` and `CODEX_PROFILES_HOME`.
- Discover profile-local `.profile.json` metadata as a fallback.
- Display profile name, aliases, expected email, family, configuration state,
  and credential-file presence.
- Generate safe `pcodex` commands for login, status, launch, and logout.
- Keep all application permissions minimal; the frontend has no arbitrary
  filesystem or shell access.

The generated commands use the existing workBenches launcher:

```bash
pcodex login PROFILE
pcodex status PROFILE
pcodex PROFILE
pcodex logout PROFILE
```

## Development

Requirements:

- Node.js 22 or newer
- pnpm 11
- Rust 1.88 or newer
- Tauri 2 system dependencies for your operating system

```bash
pnpm install
pnpm test
pnpm build
cargo test -p opensoft-profile-core
pnpm tauri dev
```

The workBenches `rust-bench` contains the Node and Rust toolchains. From that
container, this checkout is normally visible at:

```bash
cd /workspace/projects/profile-switcher
COREPACK_HOME="$HOME/.cache/corepack" corepack pnpm install
```

## Profile locations

| Purpose      | Default                                      | Override                                                 |
| ------------ | -------------------------------------------- | -------------------------------------------------------- |
| Manifest     | `~/.config/workbenches/openai-profiles.json` | `CODEX_PROFILES_MANIFEST` or `CHATGPT_PROFILES_MANIFEST` |
| Profile home | `~/.chatgpt-profiles`                        | `CODEX_PROFILES_HOME` or `CHATGPT_PROFILES_HOME`         |

`profilePath` values must be relative and may not contain `.` or `..`
components. Credential state is determined with filesystem metadata only.

## Security model

- The Tauri frontend can call only two purpose-built commands: inventory
  discovery and command generation.
- No shell, filesystem, dialog, updater, or networking plugin is enabled.
- Profile names are resolved against discovered inventory before a command is
  generated.
- Generated commands are displayed and copied for execution in the user's
  trusted terminal; this first release does not spawn authentication flows.
- Real account manifests and all credential material remain outside Git.

See [SECURITY.md](SECURITY.md) for reporting and operational guidance.

## Independence

This project is an independent Opensoft implementation based on workBenches
profile conventions. Do not copy source code, visual assets, documentation, or
branding from unlicensed account-switching projects.

## License

Apache License 2.0. See [LICENSE](LICENSE).
