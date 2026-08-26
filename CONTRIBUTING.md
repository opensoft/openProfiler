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
cargo test -p opensoft-open-profiler-broker
cargo clippy -p opensoft-open-profiler-broker --all-targets -- -D warnings
```

Changes to the credential broker must also preserve its declared CLI surface in
[docs/broker-cli.md](docs/broker-cli.md), which a consumer's stored binding is
written against. Update the declaration in the same change as the code.

Do not add copied source, visual assets, text, or branding from projects that
do not grant an applicable license.
