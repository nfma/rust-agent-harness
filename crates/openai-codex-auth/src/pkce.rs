use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{TryRng, rngs::SysRng};
use sha2::{Digest, Sha256};

pub(crate) struct AuthSecrets {
    pub(crate) state: String,
    pub(crate) verifier: String,
    pub(crate) challenge: String,
}

impl AuthSecrets {
    pub(crate) fn generate() -> Self {
        let mut state_bytes = [0_u8; 32];
        let mut verifier_bytes = [0_u8; 32];
        SysRng
            .try_fill_bytes(&mut state_bytes)
            .expect("system random number generator unavailable");
        SysRng
            .try_fill_bytes(&mut verifier_bytes)
            .expect("system random number generator unavailable");

        let state = URL_SAFE_NO_PAD.encode(state_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        Self {
            state,
            verifier,
            challenge,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed(state: &str, verifier: &str) -> Self {
        Self {
            state: state.to_owned(),
            verifier: verifier.to_owned(),
            challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_values_are_random_and_pkce_is_s256() {
        let first = AuthSecrets::generate();
        let second = AuthSecrets::generate();

        assert_ne!(first.state, second.state);
        assert_ne!(first.verifier, second.verifier);
        assert_eq!(first.state.len(), 43);
        assert_eq!(first.verifier.len(), 43);
        assert_eq!(
            first.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(first.verifier.as_bytes()))
        );
    }
}
