pub(crate) const SERVICE: &str = "rust-agent-harness";
pub(crate) const ACCOUNT: &str = "openai-codex:default";

pub(crate) trait CredentialStore {
    fn replace(&mut self, record: &[u8]) -> Result<(), StoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StoreError;

#[cfg(target_os = "macos")]
pub(crate) struct MacOsKeychain;

#[cfg(target_os = "macos")]
impl CredentialStore for MacOsKeychain {
    fn replace(&mut self, record: &[u8]) -> Result<(), StoreError> {
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, record)
            .map_err(|_| StoreError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_coordinates_are_harness_owned() {
        assert_eq!(SERVICE, "rust-agent-harness");
        assert_eq!(ACCOUNT, "openai-codex:default");
    }
}
