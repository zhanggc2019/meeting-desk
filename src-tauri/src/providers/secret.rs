use std::fmt;

/// An API credential wrapper that cannot be serialized and always redacts debug output.
pub struct ProviderCredential {
    bytes: Vec<u8>,
}

impl ProviderCredential {
    /// Takes ownership of a credential supplied by the trusted caller.
    pub fn new(value: String) -> Self {
        Self {
            bytes: value.into_bytes(),
        }
    }

    /// Returns whether the caller supplied an empty credential.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Exposes the credential only to the transport immediately before header injection.
    pub(crate) fn expose_secret(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.bytes)
    }
}

impl fmt::Debug for ProviderCredential {
    /// Redacts the credential value from debug formatting.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderCredential([REDACTED])")
    }
}

impl Drop for ProviderCredential {
    /// Best-effort clears the owned credential buffer before releasing it.
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderCredential;

    /// Verifies that debug formatting never includes the secret value.
    #[test]
    fn debug_output_is_redacted() {
        let dummy_value = "x".repeat(32);
        let credential = ProviderCredential::new(dummy_value.clone());
        let output = format!("{credential:?}");
        assert!(!output.contains(&dummy_value));
        assert!(output.contains("REDACTED"));
    }
}
