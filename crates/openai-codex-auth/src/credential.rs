use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::oauth::TokenResponse;

const AUTH_CLAIM: &str = "https://api.openai.com/auth";
const SCHEMA_VERSION: u8 = 1;

pub(crate) trait Clock {
    fn unix_seconds(&self) -> u64;
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[derive(Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct CredentialRecord {
    schema_version: u8,
    access_token: String,
    refresh_token: String,
    id_token: String,
    chatgpt_account_id: String,
    access_token_expires_at: Option<u64>,
    email: Option<String>,
    plan: Option<String>,
}

impl CredentialRecord {
    pub(crate) fn validate(response: TokenResponse, now: u64) -> Result<Self, CredentialError> {
        if response.access_token.trim().is_empty()
            || response.refresh_token.trim().is_empty()
            || response.id_token.trim().is_empty()
        {
            return Err(CredentialError::MalformedTokenSet);
        }

        let id_claims =
            decode_jwt_payload(&response.id_token).ok_or(CredentialError::MalformedTokenSet)?;
        let access_claims = decode_jwt_payload(&response.access_token);
        let id_account = account_id(&id_claims);
        let access_account = access_claims.as_ref().and_then(account_id);

        if id_account.is_some() && access_account.is_some() && id_account != access_account {
            return Err(CredentialError::MismatchedAccount);
        }
        let chatgpt_account_id = id_account
            .or(access_account)
            .ok_or(CredentialError::MissingAccount)?;

        let token_expiry = access_claims.as_ref().and_then(expiry);
        let response_expiry = response
            .expires_in
            .filter(|seconds| *seconds > 0)
            .and_then(|seconds| now.checked_add(seconds as u64));
        let usable_token_expiry = token_expiry.filter(|expiry| *expiry > now);
        if (response.expires_in.is_some() || token_expiry.is_some())
            && response_expiry.is_none()
            && usable_token_expiry.is_none()
        {
            return Err(CredentialError::InvalidExpiry);
        }

        let email = safe_claim(id_claims.get("email"), 254);
        let plan = id_claims
            .get(AUTH_CLAIM)
            .and_then(Value::as_object)
            .and_then(|claim| claim.get("chatgpt_plan_type"))
            .and_then(|value| safe_claim(Some(value), 64));

        Ok(Self {
            schema_version: SCHEMA_VERSION,
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            id_token: response.id_token,
            chatgpt_account_id,
            access_token_expires_at: response_expiry.or(usable_token_expiry),
            email,
            plan,
        })
    }

    pub(crate) fn serialize(&self) -> Result<Vec<u8>, CredentialError> {
        serde_json::to_vec(self).map_err(|_| CredentialError::Serialization)
    }

    pub(crate) fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    pub(crate) fn plan(&self) -> Option<&str> {
        self.plan.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialError {
    MalformedTokenSet,
    MissingAccount,
    MismatchedAccount,
    InvalidExpiry,
    Serialization,
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let mut segments = token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let signature = segments.next()?;
    if header.is_empty() || payload.is_empty() || signature.is_empty() || segments.next().is_some()
    {
        return None;
    }

    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&payload).ok()
}

fn account_id(claims: &Value) -> Option<String> {
    claims
        .get(AUTH_CLAIM)?
        .get("chatgpt_account_id")?
        .as_str()
        .and_then(|account| {
            let account = account.trim();
            (!account.is_empty()).then(|| account.to_owned())
        })
}

fn expiry(claims: &Value) -> Option<u64> {
    claims.get("exp")?.as_u64()
}

fn safe_claim(value: Option<&Value>, maximum_chars: usize) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty()
        || value.chars().count() > maximum_chars
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_owned())
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;

    use super::*;

    fn jwt(payload: Value) -> String {
        format!(
            "{}.{}.signature",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    fn response(id_payload: Value, access_payload: Value) -> TokenResponse {
        TokenResponse {
            access_token: jwt(access_payload),
            refresh_token: "refresh-token".to_owned(),
            id_token: jwt(id_payload),
            expires_in: Some(3600),
        }
    }

    #[test]
    fn version_one_record_round_trips() {
        let response = response(
            json!({
                AUTH_CLAIM: {
                    "chatgpt_account_id": "account-one",
                    "chatgpt_plan_type": "plus"
                },
                "email": "nuno@example.com"
            }),
            json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}, "exp": 5_000}),
        );
        let record = CredentialRecord::validate(response, 1_000).unwrap();
        let serialized = record.serialize().unwrap();
        let decoded: CredentialRecord = serde_json::from_slice(&serialized).unwrap();

        assert!(record == decoded);
        assert_eq!(decoded.schema_version, 1);
        assert_eq!(decoded.email(), Some("nuno@example.com"));
        assert_eq!(decoded.plan(), Some("plus"));
        assert_eq!(decoded.access_token_expires_at, Some(4_600));
    }

    #[test]
    fn rejects_missing_or_mismatched_account_ids() {
        let missing = response(json!({}), json!({"exp": 5_000}));
        assert_eq!(
            CredentialRecord::validate(missing, 1_000).err(),
            Some(CredentialError::MissingAccount)
        );

        let mismatched = response(
            json!({AUTH_CLAIM: {"chatgpt_account_id": "one"}}),
            json!({AUTH_CLAIM: {"chatgpt_account_id": "two"}, "exp": 5_000}),
        );
        assert_eq!(
            CredentialRecord::validate(mismatched, 1_000).err(),
            Some(CredentialError::MismatchedAccount)
        );
    }

    #[test]
    fn rejects_malformed_tokens_and_invalid_expiry() {
        let malformed = TokenResponse {
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            id_token: "not-a-jwt".to_owned(),
            expires_in: Some(3600),
        };
        assert_eq!(
            CredentialRecord::validate(malformed, 1_000).err(),
            Some(CredentialError::MalformedTokenSet)
        );

        let expired = TokenResponse {
            expires_in: Some(0),
            ..response(
                json!({AUTH_CLAIM: {"chatgpt_account_id": "one"}}),
                json!({"exp": 999}),
            )
        };
        assert_eq!(
            CredentialRecord::validate(expired, 1_000).err(),
            Some(CredentialError::InvalidExpiry)
        );
    }

    #[test]
    fn unsafe_display_metadata_is_discarded() {
        let response = response(
            json!({
                AUTH_CLAIM: {
                    "chatgpt_account_id": "one",
                    "chatgpt_plan_type": "plus\nforged"
                },
                "email": "nuno@example.com\nforged"
            }),
            json!({"exp": 5_000}),
        );
        let record = CredentialRecord::validate(response, 1_000).unwrap();

        assert_eq!(record.email(), None);
        assert_eq!(record.plan(), None);
    }
}
