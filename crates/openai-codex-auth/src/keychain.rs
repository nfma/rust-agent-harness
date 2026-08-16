pub(crate) const SERVICE: &str = "rust-agent-harness";
pub(crate) const ACCOUNT: &str = "openai-codex:default";
#[cfg(target_os = "macos")]
const ITEM_NOT_FOUND: i32 = -25_300;

pub(crate) trait CredentialStore {
    fn replace(&mut self, record: &[u8]) -> Result<(), StoreError>;
}

pub(crate) trait CredentialReader {
    fn read(&self) -> Result<Option<Vec<u8>>, StoreError>;
}

pub(crate) trait CredentialDeleter {
    fn delete(&self) -> Result<DeleteOutcome, StoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeleteOutcome {
    Deleted,
    Absent,
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

#[cfg(target_os = "macos")]
impl CredentialReader for MacOsKeychain {
    fn read(&self) -> Result<Option<Vec<u8>>, StoreError> {
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(record) => Ok(Some(record)),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
            Err(_) => Err(StoreError),
        }
    }
}

#[cfg(target_os = "macos")]
impl CredentialDeleter for MacOsKeychain {
    fn delete(&self) -> Result<DeleteOutcome, StoreError> {
        classify_delete_result(
            security_framework::passwords::delete_generic_password(SERVICE, ACCOUNT)
                .map_err(|error| error.code()),
        )
    }
}

#[cfg(target_os = "macos")]
fn classify_delete_result(result: Result<(), i32>) -> Result<DeleteOutcome, StoreError> {
    match result {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(ITEM_NOT_FOUND) => Ok(DeleteOutcome::Absent),
        Err(_) => Err(StoreError),
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

    #[test]
    fn production_delete_uses_one_exact_coordinate_call_without_reading() {
        let source = include_str!("keychain.rs");
        let (_, implementation) = source
            .split_once("impl CredentialDeleter for MacOsKeychain {")
            .expect("production delete implementation should exist");
        let (implementation, _) = implementation
            .split_once("\n}\n")
            .expect("production delete implementation should be bounded");

        assert_eq!(
            implementation
                .matches("delete_generic_password(SERVICE, ACCOUNT)")
                .count(),
            1
        );
        for forbidden in [
            "get_generic_password",
            "set_generic_password",
            "delete_generic_password_options",
            "CredentialRecord",
            "AuthorizedCredential",
            "Vec<u8>",
            "&[u8]",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "production delete must not contain {forbidden}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn delete_result_normalizes_only_item_not_found() {
        assert_eq!(classify_delete_result(Ok(())), Ok(DeleteOutcome::Deleted));
        assert_eq!(
            classify_delete_result(Err(ITEM_NOT_FOUND)),
            Ok(DeleteOutcome::Absent)
        );
        assert_eq!(classify_delete_result(Err(-1)), Err(StoreError));
    }
}
