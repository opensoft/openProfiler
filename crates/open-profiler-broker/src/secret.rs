use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};
use std::fmt;
use std::ptr;
use std::sync::atomic::{compiler_fence, Ordering};

/// Credential material held in memory.
///
/// The type exists so that credential material cannot leak through the two
/// routes it usually leaks through: a `Debug` or `Display` rendering that ends
/// up in a log line or an error, and a buffer that outlives its use. `Debug`
/// is redacted, `Display` is not implemented at all, and `Drop` overwrites the
/// backing bytes with volatile writes so the optimizer may not elide them.
///
/// `Serialize` is implemented because exactly two writes must carry the value:
/// the custody entry, and the minted token on standard output. Nothing else in
/// this crate serializes a type that holds one.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Takes ownership of the string's allocation rather than copying it, so
    /// the only buffer holding the value is the one that gets overwritten.
    pub fn from_string(value: String) -> Self {
        Self(value.into_bytes())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Removes a single trailing line ending, which a terminal or a piping
    /// caller adds and a provider credential never contains.
    pub fn trim_one_trailing_newline(mut self) -> Self {
        if self.0.last() == Some(&b'\n') {
            self.overwrite_tail(1);
            self.0.pop();
        }
        if self.0.last() == Some(&b'\r') {
            self.overwrite_tail(1);
            self.0.pop();
        }
        self
    }

    fn overwrite_tail(&mut self, count: usize) {
        let start = self.0.len().saturating_sub(count);
        for byte in &mut self.0[start..] {
            unsafe { ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe { ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.as_str() {
            Some(value) => serializer.serialize_str(value),
            None => Err(serde::ser::Error::custom(
                "credential material is not valid UTF-8",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_rendering_is_redacted() {
        let secret = Secret::from_string("sk-live-must-not-appear".to_string());
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn debug_rendering_of_a_wrapper_is_redacted() {
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            value: Secret,
        }

        let holder = Holder {
            value: Secret::from_string("sk-live-must-not-appear".to_string()),
        };
        let rendered = format!("{holder:?}");
        assert!(!rendered.contains("sk-live-must-not-appear"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn trims_exactly_one_trailing_line_ending() {
        assert_eq!(
            Secret::from_string("value\r\n".to_string())
                .trim_one_trailing_newline()
                .as_str(),
            Some("value")
        );
        assert_eq!(
            Secret::from_string("value\n\n".to_string())
                .trim_one_trailing_newline()
                .as_str(),
            Some("value\n")
        );
        assert_eq!(
            Secret::from_string("value".to_string())
                .trim_one_trailing_newline()
                .as_str(),
            Some("value")
        );
    }

    #[test]
    fn serializes_as_a_plain_string() {
        let secret = Secret::from_string("abc".to_string());
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"abc\"");
    }

    #[test]
    fn round_trips_through_json() {
        let secret: Secret = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(secret.as_str(), Some("abc"));
        assert_eq!(secret.len(), 3);
        assert!(!secret.is_empty());
    }
}
