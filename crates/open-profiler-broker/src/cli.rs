//! The `openprofiler-broker` command line.
//!
//! The surface here is a declared contract, not an implementation detail: a
//! consumer stores an argv template naming these subcommands and flags, and
//! `docs/broker-cli.md` is the document it is written against. Changing a flag
//! name changes that contract.

use crate::secret::Secret;
use crate::store::CustodyStore;
use crate::{
    ids, AuthKind, BrokerError, IntakeRequest, MintRequest, Result, DEFAULT_LIFETIME_SECONDS,
    ERROR_KIND, MAX_LIFETIME_SECONDS, MIN_LIFETIME_SECONDS, SCHEMA_VERSION,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{Read, Write};

const MAX_SECRET_BYTES: u64 = 64 * 1024;

/// Flag names that would carry credential material on argv. They are rejected
/// on every command rather than merely unimplemented, because argv is
/// world-readable and a caller reaching for one has misread the contract.
const SECRET_BEARING_FLAGS: &[&str] = &[
    "secret",
    "api-key",
    "apikey",
    "key",
    "token",
    "access-token",
    "refresh-token",
    "credential",
    "password",
    "passphrase",
];

pub const USAGE: &str = "\
openprofiler-broker — credential custody and short-lived token minting

Usage:
  openprofiler-broker <command> [options]

Commands:
  intake      take a secret on stdin into custody and print a reference
  mint        issue a short-lived token for a held credential
  revoke      destroy a held credential
  list        print the non-secret reference index
  authorize   begin a provider OAuth authorization  (DECLARED-DESIGN, not built)
  help        print this message

  intake  --binding <id> --provider <name> --auth-kind api_key|oauth
          --approved-by <principal> [--label <text>] [--issued-by <principal>]
          [--lifetime-seconds <30..3600>]
          The secret is read from standard input. It is never accepted on argv.

  mint    --reference <opref-...> [--scope <scope>]...
          [--lifetime-seconds <30..3600>] [--retry-of <opaud-...>]

  revoke  --reference <opref-...>

  list

Output:
  Success writes one JSON object to stdout and exits 0.
  Failure writes one JSON object to stderr, leaves stdout empty, and exits
  non-zero: 2 usage, 3 unknown reference, 4 credential unusable,
  5 not implemented, 6 custody unavailable, 1 internal.

Custody store:
  OPENPROFILER_BROKER_HOME, defaulting to ~/.openprofiler/broker

The full declaration, including what a minted token carries per auth kind, is
docs/broker-cli.md in the openProfiler repository.";

#[derive(Serialize)]
struct ErrorBody<'a> {
    schema_version: u32,
    kind: &'a str,
    code: &'a str,
    message: String,
    exit_code: i32,
}

/// Runs one invocation. `args` excludes the program name.
///
/// Returns the process exit code. Standard output carries the success object
/// and nothing else; standard error carries the error object and nothing else.
pub fn run(
    args: &[String],
    input: &mut dyn Read,
    output: &mut dyn Write,
    errors: &mut dyn Write,
) -> i32 {
    match execute(args, input, output) {
        Ok(()) => 0,
        Err(error) => {
            let exit_code = error.exit_code();
            let body = ErrorBody {
                schema_version: SCHEMA_VERSION,
                kind: ERROR_KIND,
                code: error.code(),
                message: error.to_string(),
                exit_code,
            };
            // Falls back to a fixed line rather than panicking: a broker that
            // cannot render its refusal must still refuse.
            let rendered = serde_json::to_string(&body).unwrap_or_else(|_| {
                format!(
                    "{{\"schema_version\":{SCHEMA_VERSION},\"kind\":\"{ERROR_KIND}\",\
                     \"code\":\"internal\",\"message\":\"the broker failed and could not \
                     render the reason\",\"exit_code\":1}}"
                )
            });
            let _ = writeln!(errors, "{rendered}");
            let _ = errors.flush();
            exit_code
        }
    }
}

fn execute(args: &[String], input: &mut dyn Read, output: &mut dyn Write) -> Result<()> {
    let Some(command) = args.first() else {
        return Err(BrokerError::usage(
            "no command given; expected intake, mint, revoke, list, authorize or help",
        ));
    };
    let rest = &args[1..];

    if matches!(command.as_str(), "help" | "--help" | "-h") {
        return write_line(output, USAGE);
    }
    if matches!(command.as_str(), "version" | "--version" | "-V") {
        return write_line(output, env!("CARGO_PKG_VERSION"));
    }
    if rest.iter().any(|arg| arg == "--help" || arg == "-h") {
        return write_line(output, USAGE);
    }

    match command.as_str() {
        "intake" => intake_command(rest, input, output),
        "mint" => mint_command(rest, output),
        "revoke" => revoke_command(rest, output),
        "list" => list_command(rest, output),
        "authorize" => authorize_command(rest),
        other => Err(BrokerError::usage(format!(
            "unknown command {other:?}; expected intake, mint, revoke, list, authorize or help"
        ))),
    }
}

fn intake_command(args: &[String], input: &mut dyn Read, output: &mut dyn Write) -> Result<()> {
    let flags = Flags::parse(
        args,
        &[
            "binding",
            "provider",
            "auth-kind",
            "approved-by",
            "label",
            "issued-by",
            "lifetime-seconds",
        ],
        &[],
    )?;

    let binding = flags.require_binding("binding")?;
    let provider = flags.require_provider("provider")?;
    let auth_kind = AuthKind::parse(&flags.require("auth-kind")?)?;
    let approved_by = flags.require_principal("approved-by")?;
    let label = flags.optional_free_text("label", ids::MAX_LABEL_LEN)?;
    let issued_by = match flags.optional_principal("issued-by")? {
        Some(value) => value,
        None => crate::broker_identity(),
    };
    let max_lifetime_seconds = flags
        .optional_lifetime("lifetime-seconds")?
        .unwrap_or(DEFAULT_LIFETIME_SECONDS);

    // Refused before standard input is touched: an OAuth grant must not reach
    // this process at all on a path that cannot store it correctly.
    if auth_kind == AuthKind::Oauth {
        return Err(BrokerError::NotImplemented(
            "the OAuth path is declared but not built: an OAuth grant is not taken on \
             standard input; use `authorize`, whose browser flow is declared in \
             docs/broker-cli.md and is not built yet"
                .to_string(),
        ));
    }

    let secret = read_secret(input)?;
    let store = CustodyStore::from_env()?;
    let receipt = crate::intake(
        &store,
        IntakeRequest {
            binding,
            provider,
            auth_kind,
            label,
            issued_by,
            approved_by,
            max_lifetime_seconds,
        },
        secret,
    )?;
    write_json(output, &receipt)
}

fn mint_command(args: &[String], output: &mut dyn Write) -> Result<()> {
    let flags = Flags::parse(
        args,
        &["reference", "scope", "lifetime-seconds", "retry-of"],
        &["scope"],
    )?;

    let reference = flags.require_reference("reference")?;
    let scope = flags.scopes("scope")?;
    let lifetime_seconds = flags.optional_lifetime("lifetime-seconds")?;
    let retry_of =
        match flags.optional("retry-of") {
            Some(value) if ids::is_valid_audit_ref(&value) => Some(value),
            Some(_) => return Err(BrokerError::usage(
                "--retry-of is not an audit reference; expected opaud- followed by 24 hex digits",
            )),
            None => None,
        };

    let store = CustodyStore::from_env()?;
    let minted = crate::mint(
        &store,
        MintRequest {
            reference,
            scope,
            lifetime_seconds,
            retry_of,
        },
    )?;
    write_json(output, &minted)
}

fn revoke_command(args: &[String], output: &mut dyn Write) -> Result<()> {
    let flags = Flags::parse(args, &["reference"], &[])?;
    let reference = flags.require_reference("reference")?;
    let store = CustodyStore::from_env()?;
    let revocation = crate::revoke(&store, &reference)?;
    write_json(output, &revocation)
}

fn list_command(args: &[String], output: &mut dyn Write) -> Result<()> {
    Flags::parse(args, &[], &[])?;
    let store = CustodyStore::from_env()?;
    let listing = crate::list(&store)?;
    write_json(output, &listing)
}

fn authorize_command(args: &[String]) -> Result<()> {
    // The surface is validated so a consumer's binding can be exercised today
    // and get a refusal that is about the missing flow rather than about a
    // typo in the invocation it will keep using once the flow lands.
    let flags = Flags::parse(
        args,
        &[
            "binding",
            "provider",
            "approved-by",
            "label",
            "scope",
            "lifetime-seconds",
        ],
        &["scope"],
    )?;
    flags.require_binding("binding")?;
    flags.require_provider("provider")?;
    flags.require_principal("approved-by")?;
    flags.optional_free_text("label", ids::MAX_LABEL_LEN)?;
    flags.scopes("scope")?;
    flags.optional_lifetime("lifetime-seconds")?;

    Err(BrokerError::NotImplemented(
        "the OAuth path is declared but not built: `authorize` will bind a loopback \
         redirect target, run the browser authorization with PKCE, and take the refresh \
         grant into custody. The flow is declared in docs/broker-cli.md"
            .to_string(),
    ))
}

fn read_secret(input: &mut dyn Read) -> Result<Secret> {
    let mut buffer = Vec::new();
    Read::take(&mut *input, MAX_SECRET_BYTES + 1)
        .read_to_end(&mut buffer)
        .map_err(|error| {
            // The message names the failure, never the bytes read so far.
            BrokerError::usage(format!(
                "could not read the credential from standard input: {}",
                error.kind()
            ))
        })?;
    let secret = Secret::new(buffer);
    if secret.len() as u64 > MAX_SECRET_BYTES {
        return Err(BrokerError::usage(format!(
            "the credential on standard input is larger than {MAX_SECRET_BYTES} bytes"
        )));
    }
    Ok(secret.trim_one_trailing_newline())
}

fn write_json<T: Serialize>(output: &mut dyn Write, value: &T) -> Result<()> {
    // Wrapped so the rendered buffer — the only place a minted token exists
    // outside the custody file — is overwritten once it has been written out.
    let rendered = Secret::from_string(
        serde_json::to_string(value)
            .map_err(|error| BrokerError::OutputUnavailable(error.to_string()))?,
    );
    output
        .write_all(rendered.as_bytes())
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(|error| BrokerError::OutputUnavailable(error.kind().to_string()))
}

fn write_line(output: &mut dyn Write, value: &str) -> Result<()> {
    writeln!(output, "{value}")
        .and_then(|()| output.flush())
        .map_err(|error| BrokerError::OutputUnavailable(error.kind().to_string()))
}

/// A tiny argv reader. Hand-written rather than pulled from an argument crate
/// because the rules it must enforce — no secret-bearing flag names, no
/// positional arguments, no silent repeats — are the point rather than a
/// side effect.
#[derive(Debug)]
struct Flags {
    values: BTreeMap<String, Vec<String>>,
}

impl Flags {
    fn parse(args: &[String], known: &[&str], repeatable: &[&str]) -> Result<Self> {
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            let Some(body) = argument.strip_prefix("--") else {
                return Err(BrokerError::usage(format!(
                    "unexpected argument {argument:?}; every value is named by a flag"
                )));
            };
            let (name, inline) = match body.split_once('=') {
                Some((name, value)) => (name.to_string(), Some(value.to_string())),
                None => (body.to_string(), None),
            };

            if SECRET_BEARING_FLAGS.contains(&name.as_str()) {
                return Err(BrokerError::usage_coded(
                    "secret_in_argv",
                    format!(
                        "--{name} is not accepted: credential material is read from standard \
                         input, never from argv, because argv is readable by other processes"
                    ),
                ));
            }
            if !known.contains(&name.as_str()) {
                return Err(BrokerError::usage(format!(
                    "unknown flag --{name} for this command"
                )));
            }

            let value = match inline {
                Some(value) => {
                    index += 1;
                    value
                }
                None => {
                    let Some(value) = args.get(index + 1) else {
                        return Err(BrokerError::usage(format!("--{name} needs a value")));
                    };
                    if value.starts_with("--") {
                        return Err(BrokerError::usage(format!("--{name} needs a value")));
                    }
                    index += 2;
                    value.clone()
                }
            };

            let entry = values.entry(name.clone()).or_default();
            if !entry.is_empty() && !repeatable.contains(&name.as_str()) {
                return Err(BrokerError::usage(format!(
                    "--{name} was given more than once"
                )));
            }
            entry.push(value);
        }
        Ok(Self { values })
    }

    fn optional(&self, name: &str) -> Option<String> {
        self.values
            .get(name)
            .and_then(|values| values.first())
            .cloned()
    }

    fn require(&self, name: &str) -> Result<String> {
        self.optional(name)
            .ok_or_else(|| BrokerError::usage(format!("--{name} is required")))
    }

    fn require_binding(&self, name: &str) -> Result<String> {
        let value = self.require(name)?;
        if !ids::is_valid_binding(&value) {
            return Err(BrokerError::usage(format!(
                "--{name} must be 1 to {} characters of letters, digits, '-', '_', '.' or ':'",
                ids::MAX_BINDING_LEN
            )));
        }
        Ok(value)
    }

    fn require_provider(&self, name: &str) -> Result<String> {
        let value = self.require(name)?;
        if !ids::is_valid_provider(&value) {
            return Err(BrokerError::usage(format!(
                "--{name} must be 1 to {} characters of letters, digits, '-', '_', '.' or ':'",
                ids::MAX_PROVIDER_LEN
            )));
        }
        Ok(value)
    }

    fn require_principal(&self, name: &str) -> Result<String> {
        let value = self.require(name)?;
        if !ids::is_valid_free_text(&value, ids::MAX_PRINCIPAL_LEN) {
            return Err(BrokerError::usage(format!(
                "--{name} must be 1 to {} printable characters naming a principal",
                ids::MAX_PRINCIPAL_LEN
            )));
        }
        Ok(value)
    }

    fn optional_principal(&self, name: &str) -> Result<Option<String>> {
        match self.optional(name) {
            Some(_) => self.require_principal(name).map(Some),
            None => Ok(None),
        }
    }

    fn optional_free_text(&self, name: &str, max_len: usize) -> Result<Option<String>> {
        match self.optional(name) {
            Some(value) if ids::is_valid_free_text(&value, max_len) => Ok(Some(value)),
            Some(_) => Err(BrokerError::usage(format!(
                "--{name} must be 1 to {max_len} printable characters"
            ))),
            None => Ok(None),
        }
    }

    fn require_reference(&self, name: &str) -> Result<String> {
        let value = self.require(name)?;
        if !ids::is_valid_reference(&value) {
            return Err(BrokerError::usage(format!(
                "--{name} is not a credential reference; expected {} followed by 24 hex digits",
                ids::REFERENCE_PREFIX
            )));
        }
        Ok(value)
    }

    fn scopes(&self, name: &str) -> Result<Vec<String>> {
        let values = self.values.get(name).cloned().unwrap_or_default();
        if values.len() > ids::MAX_SCOPE_COUNT {
            return Err(BrokerError::usage(format!(
                "--{name} was given more than {} times",
                ids::MAX_SCOPE_COUNT
            )));
        }
        for value in &values {
            if !ids::is_valid_scope(value) {
                return Err(BrokerError::usage(format!(
                    "--{name} {value:?} must be 1 to {} characters of letters, digits, \
                     '-', '_', '.', ':' or '/'",
                    ids::MAX_SCOPE_LEN
                )));
            }
        }
        Ok(values)
    }

    fn optional_lifetime(&self, name: &str) -> Result<Option<u64>> {
        let Some(value) = self.optional(name) else {
            return Ok(None);
        };
        let seconds: u64 = value.parse().map_err(|_| {
            BrokerError::usage(format!("--{name} must be a whole number of seconds"))
        })?;
        if !(MIN_LIFETIME_SECONDS..=MAX_LIFETIME_SECONDS).contains(&seconds) {
            return Err(BrokerError::usage(format!(
                "--{name} must be between {MIN_LIFETIME_SECONDS} and {MAX_LIFETIME_SECONDS}"
            )));
        }
        Ok(Some(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn parse(values: &[&str]) -> Result<Flags> {
        Flags::parse(
            &args(values),
            &["binding", "provider", "scope", "lifetime-seconds"],
            &["scope"],
        )
    }

    #[test]
    fn accepts_both_flag_spellings() {
        let flags = parse(&["--binding", "one", "--provider=two"]).unwrap();
        assert_eq!(flags.optional("binding").as_deref(), Some("one"));
        assert_eq!(flags.optional("provider").as_deref(), Some("two"));
    }

    #[test]
    fn refuses_a_flag_that_would_carry_a_secret_on_argv() {
        for spelling in [
            "--secret",
            "--api-key",
            "--apikey",
            "--key",
            "--token",
            "--credential",
            "--password",
        ] {
            let error = Flags::parse(&args(&[spelling, "value"]), &["binding"], &[])
                .expect_err("a secret-bearing flag is refused");
            assert_eq!(error.code(), "secret_in_argv", "for {spelling}");
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains("standard input"));
        }
    }

    #[test]
    fn refuses_a_secret_bearing_flag_in_its_inline_spelling() {
        let error = Flags::parse(&args(&["--secret=sk-live"]), &["binding"], &[])
            .expect_err("a secret-bearing flag is refused");
        assert_eq!(error.code(), "secret_in_argv");
        assert!(!error.to_string().contains("sk-live"));
    }

    #[test]
    fn refuses_positionals_unknown_flags_and_silent_repeats() {
        assert_eq!(parse(&["value"]).unwrap_err().exit_code(), 2);
        assert_eq!(parse(&["--nonesuch", "value"]).unwrap_err().exit_code(), 2);
        assert_eq!(
            parse(&["--binding", "one", "--binding", "two"])
                .unwrap_err()
                .exit_code(),
            2
        );
        assert_eq!(parse(&["--binding"]).unwrap_err().exit_code(), 2);
        assert_eq!(
            parse(&["--binding", "--provider", "two"])
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn collects_a_repeatable_flag() {
        let flags = parse(&["--scope", "a:b", "--scope", "c/d"]).unwrap();
        assert_eq!(flags.scopes("scope").unwrap(), vec!["a:b", "c/d"]);
    }

    #[test]
    fn bounds_the_lifetime() {
        assert_eq!(
            parse(&["--lifetime-seconds", "300"])
                .unwrap()
                .optional_lifetime("lifetime-seconds")
                .unwrap(),
            Some(300)
        );
        for bad in ["0", "29", "3601", "abc", "-5"] {
            assert!(parse(&["--lifetime-seconds", bad])
                .unwrap()
                .optional_lifetime("lifetime-seconds")
                .is_err());
        }
    }

    #[test]
    fn rejects_a_reference_that_is_not_a_reference() {
        let flags = Flags::parse(
            &args(&["--reference", "../../etc/passwd"]),
            &["reference"],
            &[],
        )
        .unwrap();
        let error = flags.require_reference("reference").unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn help_and_version_succeed_without_touching_the_store() {
        for invocation in [vec!["help"], vec!["--help"], vec!["-h"]] {
            let mut output = Vec::new();
            let mut errors = Vec::new();
            let code = run(&args(&invocation), &mut &[][..], &mut output, &mut errors);
            assert_eq!(code, 0);
            assert!(String::from_utf8(output)
                .unwrap()
                .contains("openprofiler-broker"));
            assert!(errors.is_empty());
        }

        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(
            run(
                &args(&["--version"]),
                &mut &[][..],
                &mut output,
                &mut errors
            ),
            0
        );
        assert_eq!(
            String::from_utf8(output).unwrap().trim(),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn an_unknown_command_refuses_with_a_usage_object_on_stderr() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let code = run(&args(&["dispatch"]), &mut &[][..], &mut output, &mut errors);
        assert_eq!(code, 2);
        assert!(output.is_empty(), "stdout must stay empty on failure");
        let body: serde_json::Value = serde_json::from_slice(&errors).unwrap();
        assert_eq!(body["kind"], ERROR_KIND);
        assert_eq!(body["code"], "usage");
        assert_eq!(body["exit_code"], 2);
    }

    #[test]
    fn no_command_refuses() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(run(&[], &mut &[][..], &mut output, &mut errors), 2);
        assert!(output.is_empty());
    }

    #[test]
    fn authorize_validates_its_surface_then_refuses_as_not_implemented() {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let code = run(
            &args(&[
                "authorize",
                "--binding",
                "anthropic-subscription",
                "--provider",
                "anthropic",
                "--approved-by",
                "brett@opensoft.one",
            ]),
            &mut &[][..],
            &mut output,
            &mut errors,
        );
        assert_eq!(code, 5);
        assert!(output.is_empty());
        let body: serde_json::Value = serde_json::from_slice(&errors).unwrap();
        assert_eq!(body["code"], "not_implemented");

        // A malformed invocation of the staged surface is still a usage error,
        // so a consumer can get its binding right before the flow lands.
        let mut errors = Vec::new();
        let code = run(
            &args(&["authorize", "--provider", "anthropic"]),
            &mut &[][..],
            &mut Vec::new(),
            &mut errors,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn reads_a_secret_from_standard_input_and_trims_one_newline() {
        let mut input = &b"sk-live-value\n"[..];
        let secret = read_secret(&mut input).unwrap();
        assert_eq!(secret.as_str(), Some("sk-live-value"));
    }

    #[test]
    fn refuses_a_secret_larger_than_the_declared_bound() {
        let oversized = vec![b'a'; (MAX_SECRET_BYTES + 1) as usize];
        let mut input = &oversized[..];
        let error = read_secret(&mut input).expect_err("an oversized secret is refused");
        assert_eq!(error.exit_code(), 2);
        assert!(!error.to_string().contains("aaaa"));
    }

    #[test]
    fn the_usage_text_names_every_command_it_dispatches() {
        for command in ["intake", "mint", "revoke", "list", "authorize"] {
            assert!(USAGE.contains(command), "usage omits {command}");
        }
    }
}
