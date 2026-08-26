//! End-to-end tests against the real binary.
//!
//! These spawn `openprofiler-broker`, feed it a secret on standard input, and
//! assert the declared contract in `docs/broker-cli.md`: the JSON shapes, the
//! exit codes, and the property the whole design rests on — that the secret
//! appears nowhere except the one custody file.

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

const BROKER: &str = env!("CARGO_BIN_EXE_openprofiler-broker");

/// A credential value that exists only for the duration of one test.
///
/// Assembled at runtime rather than written as a literal, and unique per run.
/// A literal would make the tracked-tree assertion below vacuous — it would
/// always match this file — and a real secret is exactly this: a value that
/// lives in no source file. So the canary behaves like one.
fn canary() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_nanos();
    format!(
        "sk-{}-{}-{}-{}-{}",
        "canary",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed),
        "must-not-appear"
    )
}

struct Broker {
    home: TempDir,
    secret: String,
}

impl Broker {
    fn new() -> Self {
        Self {
            home: TempDir::new().expect("a temporary broker home"),
            secret: canary(),
        }
    }

    fn secret(&self) -> &str {
        &self.secret
    }

    fn root(&self) -> PathBuf {
        self.home.path().join("broker")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_stdin(args, b"")
    }

    fn run_with_stdin(&self, args: &[&str], stdin: &[u8]) -> Output {
        let mut child = Command::new(BROKER)
            .args(args)
            .env("OPENPROFILER_BROKER_HOME", self.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the broker binary runs");
        let written = child
            .stdin
            .as_mut()
            .expect("stdin is piped")
            .write_all(stdin);
        if let Err(error) = written {
            // The broker refuses some invocations BEFORE it reads standard
            // input — an `--auth-kind oauth` intake does exactly that, so a
            // grant never reaches a process that cannot store it correctly —
            // and then this write lands on a pipe the child has already
            // closed. That is the contract working rather than a failure, so
            // the exit code and stderr are what get asserted. The declaration
            // puts the same obligation on a real consumer.
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe,
                "unexpected failure writing to the broker's standard input"
            );
        }
        // Closing stdin is the consumer's obligation: intake reads to EOF.
        drop(child.stdin.take());
        child.wait_with_output().expect("the broker exits")
    }

    fn intake(&self, secret: &str) -> Value {
        let output = self.run_with_stdin(
            &[
                "intake",
                "--binding",
                "anthropic-default",
                "--provider",
                "anthropic",
                "--auth-kind",
                "api_key",
                "--label",
                "Anthropic team key",
                "--approved-by",
                "brett@opensoft.one",
                "--lifetime-seconds",
                "300",
            ],
            secret.as_bytes(),
        );
        succeeded(&output)
    }
}

fn succeeded(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected success, stderr was {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "a successful run writes nothing to stderr"
    );
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON object")
}

fn refused(output: &Output, exit_code: i32, code: &str) -> Value {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(
        output.stdout.is_empty(),
        "a refusal leaves stdout empty, found {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let body: Value = serde_json::from_slice(&output.stderr).expect("stderr is one JSON object");
    assert_eq!(body["kind"], "openprofiler_broker_error");
    assert_eq!(body["code"], code);
    assert_eq!(body["exit_code"], exit_code);
    body
}

fn files_containing(root: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if std::fs::read(&path)
                .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
                .unwrap_or(false)
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_full_api_key_lifecycle_runs_end_to_end() {
    let broker = Broker::new();

    let receipt = broker.intake(broker.secret());
    assert_eq!(receipt["kind"], "openprofiler_broker_intake");
    assert_eq!(receipt["schema_version"], 1);
    assert_eq!(receipt["binding"], "anthropic-default");
    assert_eq!(receipt["provider"], "anthropic");
    assert_eq!(receipt["auth_kind"], "api_key");
    assert_eq!(receipt["label"], "Anthropic team key");
    assert_eq!(receipt["approved_by"], "brett@opensoft.one");
    assert_eq!(receipt["max_lifetime_seconds"], 300);
    assert!(receipt["issued_by"]
        .as_str()
        .unwrap()
        .starts_with("openprofiler-broker/"));
    let reference = receipt["reference"].as_str().unwrap().to_string();
    assert!(reference.starts_with("opref-"));
    assert!(receipt["audit_ref"].as_str().unwrap().starts_with("opaud-"));

    let listing = succeeded(&broker.run(&["list"]));
    assert_eq!(listing["kind"], "openprofiler_broker_reference_list");
    let entries = listing["references"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["reference"], reference.as_str());
    assert_eq!(entries[0]["mint_count"], 0);
    assert!(entries[0]["last_minted_at"].is_null());
    assert!(entries[0].get("secret").is_none());
    assert!(entries[0].get("token").is_none());

    let minted = succeeded(&broker.run(&[
        "mint",
        "--reference",
        &reference,
        "--scope",
        "messages:write",
    ]));
    assert_eq!(minted["kind"], "openprofiler_broker_mint");
    assert_eq!(minted["reference"], reference.as_str());
    assert_eq!(minted["binding"], "anthropic-default");
    assert_eq!(minted["auth_kind"], "api_key");
    assert_eq!(minted["token_type"], "api_key");
    assert_eq!(minted["expires_in_seconds"], 300);
    assert_eq!(minted["scope"][0], "messages:write");
    assert_eq!(minted["approved_by"], "brett@opensoft.one");
    assert!(minted["retry_of"].is_null());
    // The honest half of the contract, in machine-readable form.
    assert_eq!(minted["enforcement"]["expiry"], "broker_bookkeeping");
    assert_eq!(minted["enforcement"]["scope"], "declared");
    // The minted token is provider-native: the stored key itself.
    assert_eq!(minted["token"], broker.secret());
    let issued_at = minted["issued_at"].as_str().unwrap();
    let expires_at = minted["expires_at"].as_str().unwrap();
    assert!(issued_at.ends_with('Z') && expires_at.ends_with('Z'));
    assert!(issued_at < expires_at);

    let listing = succeeded(&broker.run(&["list"]));
    assert_eq!(listing["references"][0]["mint_count"], 1);
    assert_eq!(listing["references"][0]["last_minted_at"], issued_at);

    let revocation = succeeded(&broker.run(&["revoke", "--reference", &reference]));
    assert_eq!(revocation["kind"], "openprofiler_broker_revocation");
    assert_eq!(revocation["revoked"], true);
    assert_eq!(revocation["reference"], reference.as_str());

    let listing = succeeded(&broker.run(&["list"]));
    assert!(listing["references"].as_array().unwrap().is_empty());

    refused(
        &broker.run(&["mint", "--reference", &reference]),
        3,
        "unknown_reference",
    );
}

#[test]
fn the_secret_appears_nowhere_outside_the_one_custody_file() {
    let broker = Broker::new();
    let receipt = broker.intake(broker.secret());
    let reference = receipt["reference"].as_str().unwrap().to_string();

    // Nothing the broker said at intake carries it.
    let intake_output = broker.run_with_stdin(
        &[
            "intake",
            "--binding",
            "second",
            "--provider",
            "anthropic",
            "--auth-kind",
            "api_key",
            "--approved-by",
            "brett@opensoft.one",
        ],
        broker.secret().as_bytes(),
    );
    assert!(!String::from_utf8_lossy(&intake_output.stdout).contains(broker.secret()));
    assert!(!String::from_utf8_lossy(&intake_output.stderr).contains(broker.secret()));

    // Exactly one file in the whole store holds it, per reference taken.
    let holders = files_containing(broker.home.path(), broker.secret());
    assert_eq!(holders.len(), 2, "found {holders:?}");
    for holder in &holders {
        assert_eq!(
            holder.parent().unwrap(),
            broker.root().join("custody"),
            "the only file holding a secret is a custody entry"
        );
    }

    // Neither the index nor the audit log carries it.
    let index = std::fs::read_to_string(broker.root().join("references.json")).unwrap();
    assert!(!index.contains(broker.secret()));
    let audit =
        std::fs::read_to_string(broker.root().join("audit").join("broker-audit.jsonl")).unwrap();
    assert!(!audit.contains(broker.secret()));

    // `list` never discloses it.
    let listing = broker.run(&["list"]);
    assert!(!String::from_utf8_lossy(&listing.stdout).contains(broker.secret()));

    // After revocation nothing in the store holds it any more.
    broker.run(&["revoke", "--reference", &reference]);
    let second = receipt_reference(&broker, "second");
    broker.run(&["revoke", "--reference", &second]);
    assert!(files_containing(broker.home.path(), broker.secret()).is_empty());

    // And it never reached this repository at all — neither the committed
    // content nor the working tree, tracked or not.
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root");
    for scope in [["grep", "--cached", "-l"], ["grep", "--untracked", "-l"]] {
        let Ok(found) = Command::new("git")
            .args(scope)
            .arg(broker.secret())
            .current_dir(repository)
            .output()
        else {
            continue;
        };
        assert!(
            found.stdout.is_empty(),
            "the canary reached the repository via {scope:?}: {}",
            String::from_utf8_lossy(&found.stdout)
        );
    }
}

fn receipt_reference(broker: &Broker, binding: &str) -> String {
    let listing = succeeded(&broker.run(&["list"]));
    listing["references"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["binding"] == binding)
        .expect("the binding is held")["reference"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_audit_log_records_one_line_per_event_with_the_accountability_fields() {
    let broker = Broker::new();
    let receipt = broker.intake(broker.secret());
    let reference = receipt["reference"].as_str().unwrap().to_string();

    let first = succeeded(&broker.run(&["mint", "--reference", &reference]));
    let second = succeeded(&broker.run(&[
        "mint",
        "--reference",
        &reference,
        "--retry-of",
        first["audit_ref"].as_str().unwrap(),
    ]));
    succeeded(&broker.run(&["revoke", "--reference", &reference]));

    let raw =
        std::fs::read_to_string(broker.root().join("audit").join("broker-audit.jsonl")).unwrap();
    let records: Vec<Value> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(records.len(), 4);
    assert_eq!(records[0]["event"], "intake");
    assert_eq!(records[1]["event"], "mint");
    assert_eq!(records[2]["event"], "mint");
    assert_eq!(records[3]["event"], "revoke");

    for record in &records {
        assert_eq!(record["kind"], "openprofiler_broker_audit_record");
        assert_eq!(record["schema_version"], 1);
        assert!(record["issued_by"]
            .as_str()
            .unwrap()
            .starts_with("openprofiler-broker/"));
        assert_eq!(record["approved_by"], "brett@opensoft.one");
        assert!(record["audit_ref"].as_str().unwrap().starts_with("opaud-"));
        assert!(record.get("token").is_none());
        assert!(record.get("secret").is_none());
    }

    // The mint is the moment a token, and therefore an expiry, exists.
    assert_eq!(records[1]["expires_at"], first["expires_at"]);
    assert_eq!(records[2]["expires_at"], second["expires_at"]);
    assert!(records[0]["expires_at"].is_null());
    assert!(records[3]["expires_at"].is_null());

    // A mid-turn re-mint is visible rather than indistinguishable.
    assert!(records[1]["retry_of"].is_null());
    assert_eq!(records[2]["retry_of"], first["audit_ref"]);
    assert_eq!(records[1]["audit_ref"], first["audit_ref"]);
}

#[test]
fn a_secret_is_never_accepted_on_argv() {
    let broker = Broker::new();
    for spelling in ["--secret", "--api-key", "--token", "--key", "--credential"] {
        let output = broker.run(&[
            "intake",
            "--binding",
            "anthropic-default",
            "--provider",
            "anthropic",
            "--auth-kind",
            "api_key",
            "--approved-by",
            "brett@opensoft.one",
            spelling,
            broker.secret(),
        ]);
        let body = refused(&output, 2, "secret_in_argv");
        assert!(!body["message"].as_str().unwrap().contains(broker.secret()));
    }
    assert!(files_containing(broker.home.path(), broker.secret()).is_empty());
}

#[test]
fn an_empty_standard_input_refuses_rather_than_storing_nothing() {
    let broker = Broker::new();
    let output = broker.run_with_stdin(
        &[
            "intake",
            "--binding",
            "anthropic-default",
            "--provider",
            "anthropic",
            "--auth-kind",
            "api_key",
            "--approved-by",
            "brett@opensoft.one",
        ],
        b"",
    );
    refused(&output, 2, "empty_secret");
    assert!(succeeded(&broker.run(&["list"]))["references"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_missing_approver_refuses_because_a_grant_without_one_is_invalid() {
    let broker = Broker::new();
    let output = broker.run_with_stdin(
        &[
            "intake",
            "--binding",
            "anthropic-default",
            "--provider",
            "anthropic",
            "--auth-kind",
            "api_key",
        ],
        broker.secret().as_bytes(),
    );
    let body = refused(&output, 2, "usage");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("--approved-by is required"));
}

#[test]
fn the_oauth_paths_refuse_honestly_with_the_declared_exit_code() {
    let broker = Broker::new();

    let output = broker.run_with_stdin(
        &[
            "intake",
            "--binding",
            "anthropic-subscription",
            "--provider",
            "anthropic",
            "--auth-kind",
            "oauth",
            "--approved-by",
            "brett@opensoft.one",
        ],
        b"a-refresh-grant",
    );
    let body = refused(&output, 5, "not_implemented");
    assert!(body["message"].as_str().unwrap().contains("authorize"));

    let output = broker.run(&[
        "authorize",
        "--binding",
        "anthropic-subscription",
        "--provider",
        "anthropic",
        "--approved-by",
        "brett@opensoft.one",
    ]);
    let body = refused(&output, 5, "not_implemented");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("docs/broker-cli.md"));

    // Nothing was taken into custody by either refusal.
    assert!(succeeded(&broker.run(&["list"]))["references"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn a_reference_cannot_reach_outside_the_custody_root() {
    let broker = Broker::new();
    broker.intake(broker.secret());
    for candidate in [
        "../../../etc/passwd",
        "opref-../../etc/passwd",
        "opref-0011223344556677889900AA",
        "opref-00112233445566778899",
    ] {
        let output = broker.run(&["mint", "--reference", candidate]);
        assert_eq!(output.status.code(), Some(2), "accepted {candidate}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn an_unheld_reference_is_distinguishable_from_a_broken_store() {
    let broker = Broker::new();
    broker.intake(broker.secret());
    let absent = "opref-000000000000000000000000";
    refused(
        &broker.run(&["mint", "--reference", absent]),
        3,
        "unknown_reference",
    );
    refused(
        &broker.run(&["revoke", "--reference", absent]),
        3,
        "unknown_reference",
    );
}

#[test]
fn mint_clamps_a_lifetime_beyond_the_credentials_maximum() {
    let broker = Broker::new();
    let receipt = broker.intake(broker.secret());
    let reference = receipt["reference"].as_str().unwrap().to_string();

    let minted = succeeded(&broker.run(&[
        "mint",
        "--reference",
        &reference,
        "--lifetime-seconds",
        "3600",
    ]));
    assert_eq!(minted["expires_in_seconds"], 300);

    let refused_output = broker.run(&[
        "mint",
        "--reference",
        &reference,
        "--lifetime-seconds",
        "999999",
    ]);
    refused(&refused_output, 2, "usage");
}

#[test]
fn list_answers_an_absent_store_with_an_empty_list_and_creates_nothing() {
    let broker = Broker::new();
    let listing = succeeded(&broker.run(&["list"]));
    assert!(listing["references"].as_array().unwrap().is_empty());
    assert!(!broker.root().exists(), "a read must not create the store");
}

#[cfg(unix)]
#[test]
fn custody_material_is_written_with_restricted_modes() {
    use std::os::unix::fs::PermissionsExt;

    let broker = Broker::new();
    let receipt = broker.intake(broker.secret());
    let reference = receipt["reference"].as_str().unwrap();
    succeeded(&broker.run(&["mint", "--reference", reference]));

    let mode = |path: PathBuf| {
        std::fs::metadata(&path)
            .unwrap_or_else(|_| panic!("{} exists", path.display()))
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(broker.root()), 0o700);
    assert_eq!(mode(broker.root().join("custody")), 0o700);
    assert_eq!(mode(broker.root().join("audit")), 0o700);
    assert_eq!(
        mode(
            broker
                .root()
                .join("custody")
                .join(format!("{reference}.json"))
        ),
        0o600
    );
    assert_eq!(mode(broker.root().join("references.json")), 0o600);
    assert_eq!(
        mode(broker.root().join("audit").join("broker-audit.jsonl")),
        0o600
    );
}

#[test]
fn help_is_printed_on_stdout_and_names_the_declaration() {
    let broker = Broker::new();
    let output = broker.run(&["help"]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("docs/broker-cli.md"));
    assert!(text.contains("OPENPROFILER_BROKER_HOME"));
    for command in ["intake", "mint", "revoke", "list", "authorize"] {
        assert!(text.contains(command));
    }
}

#[test]
fn every_subcommand_answers_version_and_help_on_the_real_binary() {
    // docs/broker-cli.md declares `--version` / `-V` and `--help` / `-h`
    // accepted on the program AND on any subcommand. A consumer asking an
    // installed broker which version it is talking to reaches for
    // `mint --version`; an unknown-flag refusal there would be a break in the
    // declared surface, so it is asserted against the real binary.
    let broker = Broker::new();
    for command in ["intake", "mint", "revoke", "list", "authorize"] {
        for spelling in ["--version", "-V"] {
            let output = broker.run(&[command, spelling]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "`{command} {spelling}` was refused: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                env!("CARGO_PKG_VERSION"),
                "`{command} {spelling}` did not print the crate version"
            );
            assert!(
                output.stderr.is_empty(),
                "`{command} {spelling}` wrote to stderr"
            );
        }

        for spelling in ["--help", "-h"] {
            let output = broker.run(&[command, spelling]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "`{command} {spelling}` was refused: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(
                text.contains("docs/broker-cli.md") && text.contains(command),
                "`{command} {spelling}` did not print the usage"
            );
        }
    }

    // Asking a broker its version must not bring a custody store into being.
    assert!(
        !broker.root().exists(),
        "a version query created a custody store"
    );
}
