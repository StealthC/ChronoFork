//! Stable state digests.
//!
//! Session equality, replay verification and snapshot integrity all depend on
//! comparing a canonical digest rather than ad-hoc field comparisons. The
//! algorithm is SHA-256 because it is widely available, collision-resistant for
//! this purpose, and not tied to a specific host platform.

use std::fmt;

use sha2::{Digest, Sha256};

/// A SHA-256 digest of canonical emulated state.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct StateHash([u8; 32]);

impl StateHash {
    /// Wraps an existing 32-byte digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Formats the digest as lowercase hexadecimal.
    #[must_use]
    pub fn to_hex(self) -> String {
        use std::fmt::Write as _;

        let mut output = String::with_capacity(64);
        for byte in self.0 {
            // Writing to a String cannot fail; the Result is discarded
            // intentionally rather than unwrapped.
            let _ = write!(output, "{byte:02x}");
        }
        output
    }
}

impl fmt::Display for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Hashes an arbitrary byte sequence with SHA-256.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> StateHash {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = [0_u8; 32];
    output.copy_from_slice(&digest);
    StateHash(output)
}

#[cfg(test)]
mod tests {
    use super::{StateHash, hash_bytes};

    #[test]
    fn hash_is_deterministic_and_sensitive_to_input() {
        let first = hash_bytes(b"chronofork");
        let second = hash_bytes(b"chronofork");
        let changed = hash_bytes(b"chronofork!");
        assert_eq!(first, second);
        assert_ne!(first, changed);
    }

    #[test]
    fn hash_display_is_lowercase_hex() {
        let hash = StateHash::from_bytes([0xab; 32]);
        assert_eq!(hash.to_string(), "ab".repeat(32));
        assert_eq!(hash.to_hex(), hash.to_string());
    }
}
