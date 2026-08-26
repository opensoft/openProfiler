use crate::ids;
use crate::secret::Secret;
use crate::{AuditRecord, AuthKind, BrokerError, ReferenceEntry, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const BROKER_HOME_ENV: &str = "OPENPROFILER_BROKER_HOME";
const CUSTODY_DIRECTORY: &str = "custody";
const INDEX_FILE: &str = "references.json";
const AUDIT_DIRECTORY: &str = "audit";
const AUDIT_FILE: &str = "broker-audit.jsonl";
const MAX_CUSTODY_BYTES: u64 = 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_AUDIT_BYTES: u64 = 64 * 1024 * 1024;
const INDEX_KIND: &str = "openprofiler_broker_reference_index";
const CUSTODY_KIND: &str = "openprofiler_broker_custody_entry";

/// The one file in the store that holds credential material.
///
/// Written at mode `0600` inside a mode-`0700` custody directory, through the
/// same fsync-and-rename discipline `activate_profile` uses for a provider
/// credential. It is deliberate reuse of that discipline rather than a second
/// pattern.
#[derive(Debug, Serialize, Deserialize)]
pub struct CustodyRecord {
    pub schema_version: u32,
    pub kind: String,
    pub reference: String,
    pub binding: String,
    pub provider: String,
    pub auth_kind: AuthKind,
    pub label: Option<String>,
    pub created_at: String,
    pub max_lifetime_seconds: u64,
    pub issued_by: String,
    pub approved_by: String,
    pub secret: Secret,
}

impl CustodyRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reference: String,
        binding: String,
        provider: String,
        auth_kind: AuthKind,
        label: Option<String>,
        created_at: String,
        max_lifetime_seconds: u64,
        issued_by: String,
        approved_by: String,
        secret: Secret,
    ) -> Self {
        Self {
            schema_version: crate::SCHEMA_VERSION,
            kind: CUSTODY_KIND.to_string(),
            reference,
            binding,
            provider,
            auth_kind,
            label,
            created_at,
            max_lifetime_seconds,
            issued_by,
            approved_by,
            secret,
        }
    }

    pub fn to_reference_entry(&self) -> ReferenceEntry {
        ReferenceEntry {
            reference: self.reference.clone(),
            binding: self.binding.clone(),
            provider: self.provider.clone(),
            auth_kind: self.auth_kind,
            label: self.label.clone(),
            created_at: self.created_at.clone(),
            max_lifetime_seconds: self.max_lifetime_seconds,
            issued_by: self.issued_by.clone(),
            approved_by: self.approved_by.clone(),
            mint_count: 0,
            last_minted_at: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ReferenceIndex {
    schema_version: u32,
    kind: String,
    references: Vec<ReferenceEntry>,
}

/// The local custody store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyStore {
    root: PathBuf,
}

impl CustodyStore {
    /// `OPENPROFILER_BROKER_HOME`, or `~/.openprofiler/broker`.
    pub fn from_env() -> Result<Self> {
        if let Some(root) = env::var_os(BROKER_HOME_ENV) {
            let root = PathBuf::from(root);
            if root.as_os_str().is_empty() {
                return Err(BrokerError::HomeUnavailable);
            }
            return Ok(Self::at(root));
        }
        let home = dirs::home_dir().ok_or(BrokerError::HomeUnavailable)?;
        Ok(Self::at(home.join(".openprofiler").join("broker")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn custody_directory(&self) -> PathBuf {
        self.root.join(CUSTODY_DIRECTORY)
    }

    pub fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }

    pub fn audit_path(&self) -> PathBuf {
        self.root.join(AUDIT_DIRECTORY).join(AUDIT_FILE)
    }

    fn custody_path(&self, reference: &str) -> Result<PathBuf> {
        if !ids::is_valid_reference(reference) {
            return Err(BrokerError::UnknownReference);
        }
        Ok(self.custody_directory().join(format!("{reference}.json")))
    }

    /// Creates the store's directories with restricted modes. Called before
    /// every write and never on a read path, so `list` against a store that
    /// does not exist yet answers with an empty list rather than creating one.
    pub fn ensure_layout(&self) -> Result<()> {
        ensure_private_directory(&self.root)?;
        ensure_private_directory(&self.custody_directory())?;
        ensure_private_directory(&self.root.join(AUDIT_DIRECTORY))
    }

    pub fn put(&self, record: &CustodyRecord) -> Result<()> {
        self.ensure_layout()?;
        let path = self.custody_path(&record.reference)?;
        let rendered = Secret::from_string(serde_json::to_string(record).map_err(|error| {
            BrokerError::CustodyWrite {
                path: path.clone(),
                message: error.to_string(),
            }
        })?);
        atomic_write(&path, rendered.as_bytes())
    }

    pub fn get(&self, reference: &str) -> Result<CustodyRecord> {
        let path = self.custody_path(reference)?;
        let Some(bytes) = read_private_file(&path, MAX_CUSTODY_BYTES)? else {
            return Err(BrokerError::UnknownReference);
        };
        let bytes = Secret::new(bytes);
        serde_json::from_slice(bytes.as_bytes()).map_err(|_| {
            BrokerError::CredentialUnusable(
                "the custody entry could not be read as a credential record".to_string(),
            )
        })
    }

    pub fn remove(&self, reference: &str) -> Result<()> {
        let path = self.custody_path(reference)?;
        reject_symlink(&path)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(BrokerError::UnknownReference)
            }
            Err(error) => Err(BrokerError::CustodyWrite {
                path,
                message: error.to_string(),
            }),
        }
    }

    /// The non-secret reference index. An absent store is an empty store.
    pub fn read_index(&self) -> Result<Vec<ReferenceEntry>> {
        let path = self.index_path();
        let Some(bytes) = read_private_file(&path, MAX_INDEX_BYTES)? else {
            return Ok(Vec::new());
        };
        let index: ReferenceIndex =
            serde_json::from_slice(&bytes).map_err(|_| BrokerError::CustodyRead {
                path,
                message: "the reference index is not a readable index record".to_string(),
            })?;
        Ok(index.references)
    }

    pub fn write_index(&self, entries: &[ReferenceEntry]) -> Result<()> {
        self.ensure_layout()?;
        let mut entries = entries.to_vec();
        entries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.reference.cmp(&right.reference))
        });
        let index = ReferenceIndex {
            schema_version: crate::SCHEMA_VERSION,
            kind: INDEX_KIND.to_string(),
            references: entries,
        };
        let path = self.index_path();
        let rendered = serde_json::to_vec(&index).map_err(|error| BrokerError::CustodyWrite {
            path: path.clone(),
            message: error.to_string(),
        })?;
        atomic_write(&path, &rendered)
    }

    pub fn upsert_index_entry(&self, entry: ReferenceEntry) -> Result<()> {
        let mut entries = self.read_index()?;
        match entries
            .iter_mut()
            .find(|existing| existing.reference == entry.reference)
        {
            Some(existing) => *existing = entry,
            None => entries.push(entry),
        }
        self.write_index(&entries)
    }

    pub fn remove_index_entry(&self, reference: &str) -> Result<()> {
        let mut entries = self.read_index()?;
        entries.retain(|entry| entry.reference != reference);
        self.write_index(&entries)
    }

    /// Appends one audit line. The store has no rewrite path for this file:
    /// append-only is a property of the code, not only of a convention.
    ///
    /// The record is rendered into one buffer, newline included, and handed to
    /// exactly one `write` call. That is what keeps the one-JSON-object-per-line
    /// contract true when several brokers append to one store at once.
    pub fn append_audit(&self, record: &AuditRecord) -> Result<()> {
        self.ensure_layout()?;
        let path = self.audit_path();
        reject_symlink(&path)?;
        let mut line = serde_json::to_vec(record).map_err(|error| BrokerError::CustodyWrite {
            path: path.clone(),
            message: error.to_string(),
        })?;
        line.push(b'\n');

        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&path)
            .map_err(|error| BrokerError::CustodyWrite {
                path: path.clone(),
                message: error.to_string(),
            })?;
        // Exactly one `write` syscall, deliberately not `write_all`.
        //
        // The file is open in append mode, so each write seeks to the end and
        // writes there as one indivisible step relative to other writers —
        // that is the guarantee `O_APPEND` gives on a regular file (and
        // `FILE_APPEND_DATA` on Windows), and it is a guarantee about a single
        // call. POSIX's atomicity-up-to-`PIPE_BUF` rule is about pipes, not
        // regular files, and is not what is relied on here. `write_all` would
        // loop on a short write, and the remainder of this record would then
        // land after whatever another broker appended in between — splicing
        // one line into the middle of another and breaking the
        // one-JSON-object-per-line contract silently.
        //
        // So a short write is reported rather than resumed. A truncated audit
        // record must be visible: a caller that believes its issuance was
        // recorded when it was recorded only in part is worse off than one
        // told the write failed.
        let written = file
            .write(&line)
            .map_err(|error| BrokerError::CustodyWrite {
                path: path.clone(),
                message: error.to_string(),
            })?;
        if written != line.len() {
            return Err(BrokerError::CustodyWrite {
                path,
                message: format!(
                    "the audit record was written short: {written} of {} bytes, \
                     so the log may hold a truncated line",
                    line.len()
                ),
            });
        }
        file.sync_all().map_err(|error| BrokerError::CustodyWrite {
            path,
            message: error.to_string(),
        })
    }

    pub fn read_audit(&self) -> Result<Vec<AuditRecord>> {
        let path = self.audit_path();
        let Some(bytes) = read_private_file(&path, MAX_AUDIT_BYTES)? else {
            return Ok(Vec::new());
        };
        String::from_utf8(bytes)
            .map_err(|_| BrokerError::CustodyRead {
                path: path.clone(),
                message: "the audit log is not valid UTF-8".to_string(),
            })?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|_| BrokerError::CustodyRead {
                    path: path.clone(),
                    message: "the audit log holds an unreadable record".to_string(),
                })
            })
            .collect()
    }
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err(BrokerError::UnsafeCustodyPath(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| BrokerError::CustodyWrite {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        Err(error) => {
            return Err(BrokerError::CustodyRead {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            BrokerError::CustodyWrite {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(BrokerError::UnsafeCustodyPath(path.to_path_buf()));
    }
    Ok(())
}

/// Reads a store file. `Ok(None)` means the file is not there, which is the
/// normal state of a store nobody has written to yet and never an error.
fn read_private_file(path: &Path, max_bytes: u64) -> Result<Option<Vec<u8>>> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BrokerError::CustodyRead {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
    };
    let length = file
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    if length > max_bytes {
        return Err(BrokerError::CustodyRead {
            path: path.to_path_buf(),
            message: "the file is larger than this store accepts".to_string(),
        });
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|error| BrokerError::CustodyRead {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    Ok(Some(bytes))
}

fn atomic_write(target: &Path, value: &[u8]) -> Result<()> {
    reject_symlink(target)?;
    let temporary = temporary_path(target);
    let mut file = secure_create(&temporary)?;
    let outcome = file
        .write_all(value)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = outcome {
        let _ = fs::remove_file(&temporary);
        return Err(BrokerError::CustodyWrite {
            path: temporary,
            message: error.to_string(),
        });
    }
    fs::rename(&temporary, target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        BrokerError::CustodyWrite {
            path: target.to_path_buf(),
            message: error.to_string(),
        }
    })
}

fn secure_create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| BrokerError::CustodyWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn temporary_path(target: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("broker");
    target.with_file_name(format!(
        ".{name}.openprofiler-broker-{}-{nonce}.tmp",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditEvent, Enforced, Enforcement};
    use tempfile::TempDir;

    fn store(temp: &TempDir) -> CustodyStore {
        CustodyStore::at(temp.path().join("broker"))
    }

    fn record(reference: &str) -> CustodyRecord {
        CustodyRecord::new(
            reference.to_string(),
            "anthropic-default".to_string(),
            "anthropic".to_string(),
            AuthKind::ApiKey,
            Some("team key".to_string()),
            "2026-08-26T14:03:11Z".to_string(),
            300,
            "openprofiler-broker/test".to_string(),
            "brett@opensoft.one".to_string(),
            Secret::from_string("sk-live-must-not-appear".to_string()),
        )
    }

    #[test]
    fn round_trips_a_custody_record() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let reference = ids::new_reference();
        store.put(&record(&reference)).unwrap();

        let read = store.get(&reference).unwrap();
        assert_eq!(read.reference, reference);
        assert_eq!(read.secret.as_str(), Some("sk-live-must-not-appear"));
        assert_eq!(read.kind, CUSTODY_KIND);
    }

    #[test]
    fn an_absent_reference_is_unknown_rather_than_an_io_failure() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        assert!(matches!(
            store.get(&ids::new_reference()),
            Err(BrokerError::UnknownReference)
        ));
        assert!(matches!(
            store.remove(&ids::new_reference()),
            Err(BrokerError::UnknownReference)
        ));
    }

    #[test]
    fn a_reference_cannot_traverse_out_of_the_custody_root() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        for candidate in ["../../etc/passwd", "opref-../../etc/passwd", "", "x"] {
            assert!(matches!(
                store.get(candidate),
                Err(BrokerError::UnknownReference)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn custody_files_and_directories_carry_restricted_modes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let reference = ids::new_reference();
        store.put(&record(&reference)).unwrap();
        store
            .write_index(&[record(&reference).to_reference_entry()])
            .unwrap();
        store.append_audit(&audit(&reference)).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.root()), 0o700);
        assert_eq!(mode(&store.custody_directory()), 0o700);
        assert_eq!(
            mode(&store.custody_directory().join(format!("{reference}.json"))),
            0o600
        );
        assert_eq!(mode(&store.index_path()), 0o600);
        assert_eq!(mode(&store.audit_path()), 0o600);
    }

    fn audit(reference: &str) -> AuditRecord {
        AuditRecord {
            schema_version: crate::SCHEMA_VERSION,
            kind: crate::AUDIT_RECORD_KIND.to_string(),
            audit_ref: ids::new_audit_ref(),
            event: AuditEvent::Mint,
            recorded_at: "2026-08-26T14:07:52Z".to_string(),
            reference: reference.to_string(),
            binding: "anthropic-default".to_string(),
            provider: "anthropic".to_string(),
            auth_kind: AuthKind::ApiKey,
            scope: vec!["messages:write".to_string()],
            issued_by: "openprofiler-broker/test".to_string(),
            approved_by: "brett@opensoft.one".to_string(),
            expires_at: Some("2026-08-26T14:12:52Z".to_string()),
            enforcement: Enforcement {
                expiry: Enforced::BrokerBookkeeping,
                scope: Enforced::Declared,
            },
            retry_of: None,
        }
    }

    #[test]
    fn the_audit_log_only_ever_grows() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let reference = ids::new_reference();
        store.append_audit(&audit(&reference)).unwrap();
        store.append_audit(&audit(&reference)).unwrap();
        store.append_audit(&audit(&reference)).unwrap();
        assert_eq!(store.read_audit().unwrap().len(), 3);
    }

    #[test]
    fn an_absent_store_reads_as_empty_and_is_not_created() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        assert!(store.read_index().unwrap().is_empty());
        assert!(store.read_audit().unwrap().is_empty());
        assert!(!store.root().exists());
    }

    #[test]
    fn the_index_is_ordered_and_upserted_in_place() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let first = ids::new_reference();
        let second = ids::new_reference();

        let mut early = record(&first).to_reference_entry();
        early.created_at = "2026-01-01T00:00:00Z".to_string();
        let mut late = record(&second).to_reference_entry();
        late.created_at = "2026-08-26T14:03:11Z".to_string();

        store.upsert_index_entry(late.clone()).unwrap();
        store.upsert_index_entry(early.clone()).unwrap();
        let entries = store.read_index().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].reference, first);

        late.mint_count = 4;
        store.upsert_index_entry(late).unwrap();
        let entries = store.read_index().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].mint_count, 4);

        store.remove_index_entry(&first).unwrap();
        assert_eq!(store.read_index().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_custody_path_is_refused() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.ensure_layout().unwrap();
        let reference = ids::new_reference();
        let elsewhere = temp.path().join("elsewhere.json");
        fs::write(&elsewhere, "{}").unwrap();
        std::os::unix::fs::symlink(
            &elsewhere,
            store.custody_directory().join(format!("{reference}.json")),
        )
        .unwrap();

        assert!(matches!(
            store.get(&reference),
            Err(BrokerError::UnsafeCustodyPath(_))
        ));
        assert!(matches!(
            store.put(&record(&reference)),
            Err(BrokerError::UnsafeCustodyPath(_))
        ));
    }
}
