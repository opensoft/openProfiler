use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const REFERENCE_PREFIX: &str = "opref-";
pub const AUDIT_PREFIX: &str = "opaud-";
const ID_HEX_LEN: usize = 24;

pub const MAX_BINDING_LEN: usize = 128;
pub const MAX_PROVIDER_LEN: usize = 64;
pub const MAX_PRINCIPAL_LEN: usize = 256;
pub const MAX_LABEL_LEN: usize = 256;
pub const MAX_SCOPE_LEN: usize = 128;
pub const MAX_SCOPE_COUNT: usize = 32;

/// A fresh custody reference.
///
/// References are unique, not unguessable: they are derived from a monotonic
/// clock, the process id and a per-process counter rather than from a CSPRNG.
/// That is deliberate and declared in `docs/broker-cli.md` — a reference is not
/// a capability. It names an entry that only the local user's own file
/// permissions protect, so guessing one grants nothing.
pub fn new_reference() -> String {
    format!("{REFERENCE_PREFIX}{}", unique_hex())
}

/// A fresh audit-record identifier, with the same properties.
pub fn new_audit_ref() -> String {
    format!("{AUDIT_PREFIX}{}", unique_hex())
}

/// Every reference accepted from argv passes through here before it is joined
/// to a path, so a reference can never traverse out of the custody root.
pub fn is_valid_reference(value: &str) -> bool {
    has_prefixed_hex_shape(value, REFERENCE_PREFIX)
}

pub fn is_valid_audit_ref(value: &str) -> bool {
    has_prefixed_hex_shape(value, AUDIT_PREFIX)
}

fn has_prefixed_hex_shape(value: &str, prefix: &str) -> bool {
    let Some(body) = value.strip_prefix(prefix) else {
        return false;
    };
    body.len() == ID_HEX_LEN
        && body
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn unique_hex() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let process = u64::from(std::process::id());
    let high = mix(nanos ^ process.rotate_left(32));
    let low = mix(high ^ count.wrapping_add(0x9E37_79B9_7F4A_7C15));
    format!("{high:016x}{:08x}", (low >> 32) as u32)
}

/// The SplitMix64 finalizer: a cheap avalanche so neighbouring inputs do not
/// produce neighbouring identifiers.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// A binding id, provider name or scope: printable, bounded, and free of
/// anything that could be read as a path component or a shell token.
pub fn is_valid_token(value: &str, max_len: usize, extra: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || extra.contains(&byte)
        })
}

pub fn is_valid_binding(value: &str) -> bool {
    is_valid_token(value, MAX_BINDING_LEN, b".:")
}

pub fn is_valid_provider(value: &str) -> bool {
    is_valid_token(value, MAX_PROVIDER_LEN, b".:")
}

pub fn is_valid_scope(value: &str) -> bool {
    is_valid_token(value, MAX_SCOPE_LEN, b".:/")
}

/// A principal or label: free text, but never a control character, because a
/// control character in a value that reaches a terminal or an append-only log
/// is how a record gets forged.
pub fn is_valid_free_text(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_len
        && !value.chars().any(|character| character.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn fresh_references_have_the_declared_shape() {
        let reference = new_reference();
        assert!(reference.starts_with(REFERENCE_PREFIX));
        assert_eq!(reference.len(), REFERENCE_PREFIX.len() + ID_HEX_LEN);
        assert!(is_valid_reference(&reference));
        assert!(!is_valid_audit_ref(&reference));
    }

    #[test]
    fn fresh_audit_references_have_the_declared_shape() {
        let audit = new_audit_ref();
        assert!(is_valid_audit_ref(&audit));
        assert!(!is_valid_reference(&audit));
    }

    #[test]
    fn references_do_not_collide_within_a_process() {
        let generated: HashSet<String> = (0..10_000).map(|_| new_reference()).collect();
        assert_eq!(generated.len(), 10_000);
    }

    #[test]
    fn rejects_references_that_could_traverse_a_path() {
        for candidate in [
            "",
            "opref-",
            "opref-../../etc/passwd",
            "opref-4f2a91c07be3d5a8140b6e7",
            "opref-4f2a91c07be3d5a8140b6e777",
            "opref-4F2A91C07BE3D5A8140B6E77",
            "opref-4f2a91c07be3d5a8140b6e7z",
            "../opref-4f2a91c07be3d5a8140b6e77",
            "opref-4f2a91c07be3d5a8140b6e77/x",
        ] {
            assert!(!is_valid_reference(candidate), "accepted {candidate:?}");
        }
    }

    #[test]
    fn validates_bindings_providers_and_scopes() {
        assert!(is_valid_binding("anthropic-default"));
        assert!(is_valid_binding("team:anthropic.default"));
        assert!(!is_valid_binding(""));
        assert!(!is_valid_binding("has space"));
        assert!(!is_valid_binding("../escape"));
        assert!(!is_valid_binding(&"a".repeat(MAX_BINDING_LEN + 1)));

        assert!(is_valid_provider("anthropic"));
        assert!(!is_valid_provider("anthropic/../openai"));

        assert!(is_valid_scope("messages:write"));
        assert!(is_valid_scope("v1/messages"));
        assert!(!is_valid_scope("messages write"));
    }

    #[test]
    fn validates_free_text() {
        assert!(is_valid_free_text("Anthropic — team key", MAX_LABEL_LEN));
        assert!(!is_valid_free_text("", MAX_LABEL_LEN));
        assert!(!is_valid_free_text("forged\nrecord", MAX_LABEL_LEN));
        assert!(!is_valid_free_text("bell\u{7}", MAX_LABEL_LEN));
        assert!(!is_valid_free_text(
            &"é".repeat(MAX_LABEL_LEN + 1),
            MAX_LABEL_LEN
        ));
    }
}
