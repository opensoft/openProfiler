# Contributing

Changes must preserve the credential boundary described in
[SECURITY.md](SECURITY.md).

Before opening a pull request:

```bash
pnpm install --frozen-lockfile
pnpm test
pnpm build
cargo fmt --all --check
cargo test -p opensoft-open-profiler-core
cargo clippy -p opensoft-open-profiler-core --all-targets -- -D warnings
```

Do not add copied source, visual assets, text, or branding from projects that
do not grant an applicable license.
