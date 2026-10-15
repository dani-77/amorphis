//! Fixed-capacity secret input with explicit erasure.

use std::error::Error;
use std::ffi::CString;
use std::fmt;

pub const DEFAULT_SECRET_CAPACITY: usize = 1024;
pub const MAX_SECRET_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretError {
    InvalidCapacity,
    TooLong,
    ContainsNul,
    AllocationFailed,
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity => f.write_str("invalid secret capacity"),
            Self::TooLong => f.write_str("secret is too long"),
            Self::ContainsNul => f.write_str("secret contains a NUL byte"),
            Self::AllocationFailed => f.write_str("secret allocation failed"),
        }
    }
}

impl Error for SecretError {}

/// A UTF-8 secret that never reallocates after construction.
///
/// The backing allocation is erased with `explicit_bzero(3)` on clear and
/// drop. Passing the secret to PAM necessarily creates copies owned by PAM;
/// this type cannot erase those external copies.
pub struct SecretBuffer {
    storage: Vec<u8>,
    used: usize,
}

impl SecretBuffer {
    pub fn new(capacity: usize) -> Result<Self, SecretError> {
        if capacity == 0 || capacity > MAX_SECRET_CAPACITY {
            return Err(SecretError::InvalidCapacity);
        }

        let mut storage = Vec::new();
        storage
            .try_reserve_exact(capacity)
            .map_err(|_| SecretError::AllocationFailed)?;
        storage.resize(capacity, 0);
        Ok(Self { storage, used: 0 })
    }

    pub fn standard() -> Result<Self, SecretError> {
        Self::new(DEFAULT_SECRET_CAPACITY)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.used
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.storage.len()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.storage[..self.used]
    }

    pub fn push_str(&mut self, text: &str) -> Result<(), SecretError> {
        if text.as_bytes().contains(&0) {
            return Err(SecretError::ContainsNul);
        }
        let end = self
            .used
            .checked_add(text.len())
            .ok_or(SecretError::TooLong)?;
        if end > self.storage.len() {
            return Err(SecretError::TooLong);
        }
        self.storage[self.used..end].copy_from_slice(text.as_bytes());
        self.used = end;
        Ok(())
    }

    /// Remove the final Unicode scalar value without leaving its bytes in the
    /// active allocation.
    pub fn pop_char(&mut self) -> bool {
        let Some((new_len, _)) = std::str::from_utf8(self.as_bytes())
            .expect("SecretBuffer only accepts UTF-8")
            .char_indices()
            .next_back()
        else {
            return false;
        };

        wipe(&mut self.storage[new_len..self.used]);
        self.used = new_len;
        true
    }

    pub fn clear(&mut self) {
        wipe(&mut self.storage);
        self.used = 0;
    }

    pub(crate) fn to_c_string(&self) -> CString {
        CString::new(self.as_bytes()).expect("SecretBuffer rejects NUL bytes")
    }
}

impl Default for SecretBuffer {
    fn default() -> Self {
        Self::standard().expect("the fixed default secret allocation must fit")
    }
}

impl fmt::Debug for SecretBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretBuffer")
            .field("used", &self.used)
            .field("capacity", &self.storage.len())
            .field("contents", &"<redacted>")
            .finish()
    }
}

impl Drop for SecretBuffer {
    fn drop(&mut self) {
        self.clear();
    }
}

fn wipe(bytes: &mut [u8]) {
    if bytes.is_empty() {
        return;
    }

    // SAFETY: `bytes` is a valid writable allocation of exactly `len`
    // bytes. `explicit_bzero` does not retain the pointer and is specifically
    // defined not to be optimized away.
    unsafe {
        libc::explicit_bzero(bytes.as_mut_ptr().cast(), bytes.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_capacity_and_nul() {
        assert_eq!(
            SecretBuffer::new(0).unwrap_err(),
            SecretError::InvalidCapacity
        );
        assert_eq!(
            SecretBuffer::new(MAX_SECRET_CAPACITY + 1).unwrap_err(),
            SecretError::InvalidCapacity
        );

        let mut secret = SecretBuffer::new(8).unwrap();
        assert_eq!(secret.push_str("a\0b"), Err(SecretError::ContainsNul));
        assert!(secret.is_empty());
    }

    #[test]
    fn refuses_growth_instead_of_reallocating() {
        let mut secret = SecretBuffer::new(4).unwrap();
        secret.push_str("1234").unwrap();
        assert_eq!(secret.push_str("5"), Err(SecretError::TooLong));
        assert_eq!(secret.as_bytes(), b"1234");
        assert_eq!(secret.capacity(), 4);
    }

    #[test]
    fn unicode_backspace_removes_one_scalar_and_erases_tail() {
        let mut secret = SecretBuffer::new(8).unwrap();
        secret.push_str("aé").unwrap();
        assert!(secret.pop_char());
        assert_eq!(secret.as_bytes(), b"a");
        assert_eq!(&secret.storage[1..3], &[0, 0]);
    }

    #[test]
    fn clear_erases_the_whole_allocation() {
        let mut secret = SecretBuffer::new(8).unwrap();
        secret.push_str("secret").unwrap();
        secret.clear();
        assert!(secret.is_empty());
        assert!(secret.storage.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let mut secret = SecretBuffer::new(16).unwrap();
        secret.push_str("do-not-print").unwrap();
        let debug = format!("{secret:?}");
        assert!(!debug.contains("do-not-print"));
        assert!(debug.contains("<redacted>"));
    }
}
