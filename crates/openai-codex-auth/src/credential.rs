#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::oauth::TokenResponse;
use crate::refresh::RefreshResponse;

const AUTH_CLAIM: &str = "https://api.openai.com/auth";
const SCHEMA_VERSION: u8 = 1;
const REFRESH_WINDOW_SECONDS: u64 = 5 * 60;

pub(crate) trait Clock {
    fn unix_seconds(&self) -> u64;
}

#[cfg(target_os = "macos")]
pub(crate) struct SystemClock;

#[cfg(target_os = "macos")]
impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
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

        let access_token_expires_at = earliest_deadline(response_expiry, usable_token_expiry);
        let (email, plan) = display_metadata(&id_claims);

        Ok(Self {
            schema_version: SCHEMA_VERSION,
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            id_token: response.id_token,
            chatgpt_account_id,
            access_token_expires_at,
            email,
            plan,
        })
    }

    pub(crate) fn serialize(&self) -> Result<Vec<u8>, CredentialError> {
        serde_json::to_vec(self).map_err(|_| CredentialError::Serialization)
    }

    pub(crate) fn deserialize(serialized: &[u8], now: u64) -> Result<Self, StoredCredentialError> {
        let record: Self =
            serde_json::from_slice(serialized).map_err(|_| StoredCredentialError::Malformed)?;
        if record.schema_version != SCHEMA_VERSION {
            return Err(StoredCredentialError::UnsupportedVersion);
        }
        if !valid_header_component(&record.access_token)
            || !valid_header_component(&record.chatgpt_account_id)
        {
            return Err(StoredCredentialError::Malformed);
        }
        if record
            .access_token_expires_at
            .is_some_and(|expiry| expiry <= now)
        {
            return Err(StoredCredentialError::Expired);
        }
        Ok(record)
    }

    pub(crate) fn deserialize_structural(serialized: &[u8]) -> Result<Self, StoredCredentialError> {
        let record: Self =
            serde_json::from_slice(serialized).map_err(|_| StoredCredentialError::Malformed)?;
        if record.schema_version != SCHEMA_VERSION {
            return Err(StoredCredentialError::UnsupportedVersion);
        }
        if !valid_header_component(&record.access_token)
            || !valid_header_component(&record.chatgpt_account_id)
            || record.refresh_token.trim().is_empty()
            || record.id_token.trim().is_empty()
        {
            return Err(StoredCredentialError::Malformed);
        }

        let access_claims = decode_jwt_payload(&record.access_token);
        let id_claims =
            decode_jwt_payload(&record.id_token).ok_or(StoredCredentialError::Malformed)?;
        for token_account in [
            access_claims.as_ref().and_then(account_id),
            account_id(&id_claims),
        ]
        .into_iter()
        .flatten()
        {
            if token_account != record.chatgpt_account_id {
                return Err(StoredCredentialError::Malformed);
            }
        }

        Ok(record)
    }

    pub(crate) fn refresh_due(&self, now: u64) -> bool {
        self.access_token_expires_at
            .is_none_or(|expiry| expiry <= now.saturating_add(REFRESH_WINDOW_SECONDS))
    }

    pub(crate) fn apply_refresh(
        &self,
        response: RefreshResponse,
        now: u64,
    ) -> Result<Self, CredentialError> {
        let access_token = required_returned_token(response.access_token)?;
        if !valid_header_component(&access_token) {
            return Err(CredentialError::MalformedTokenSet);
        }
        let access_claims =
            decode_jwt_payload(&access_token).ok_or(CredentialError::MalformedTokenSet)?;
        validate_returned_account(&access_claims, &self.chatgpt_account_id)?;

        let refresh_token = optional_rotated_token(response.refresh_token, &self.refresh_token)?;
        let (id_token, email, plan) = match response.id_token {
            Some(id_token) => {
                if id_token.trim().is_empty() {
                    return Err(CredentialError::MalformedTokenSet);
                }
                let claims =
                    decode_jwt_payload(&id_token).ok_or(CredentialError::MalformedTokenSet)?;
                validate_returned_account(&claims, &self.chatgpt_account_id)?;
                let (email, plan) = display_metadata(&claims);
                (id_token, email, plan)
            }
            None => (self.id_token.clone(), self.email.clone(), self.plan.clone()),
        };

        let token_expiry = expiry(&access_claims);
        let response_expiry = response
            .expires_in
            .filter(|seconds| *seconds > 0)
            .and_then(|seconds| now.checked_add(seconds as u64));
        let access_token_expires_at = earliest_deadline(response_expiry, token_expiry)
            .ok_or(CredentialError::InvalidExpiry)?;

        Ok(Self {
            schema_version: SCHEMA_VERSION,
            access_token,
            refresh_token,
            id_token,
            chatgpt_account_id: self.chatgpt_account_id.clone(),
            access_token_expires_at: Some(access_token_expires_at),
            email,
            plan,
        })
    }

    pub(crate) fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    pub(crate) fn authorization(&self) -> (String, String) {
        (self.access_token.clone(), self.chatgpt_account_id.clone())
    }

    pub(crate) fn into_authorization(self) -> (String, String) {
        (self.access_token, self.chatgpt_account_id)
    }

    pub(crate) fn email(&self) -> Option<&str> {
        self.email.as_deref()
    }

    pub(crate) fn plan(&self) -> Option<&str> {
        self.plan.as_deref()
    }
}

fn valid_header_component(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && !value.chars().any(char::is_control)
        && reqwest::header::HeaderValue::from_str(value).is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredCredentialError {
    UnsupportedVersion,
    Malformed,
    Expired,
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

fn validate_returned_account(
    claims: &Value,
    stored_account_id: &str,
) -> Result<(), CredentialError> {
    if account_id(claims).is_some_and(|account| account != stored_account_id) {
        Err(CredentialError::MismatchedAccount)
    } else {
        Ok(())
    }
}

fn expiry(claims: &Value) -> Option<u64> {
    claims.get("exp")?.as_u64()
}

fn earliest_deadline(first: Option<u64>, second: Option<u64>) -> Option<u64> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.min(second)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

fn required_returned_token(token: Option<String>) -> Result<String, CredentialError> {
    match token {
        Some(token) if !token.trim().is_empty() => Ok(token),
        Some(_) | None => Err(CredentialError::MalformedTokenSet),
    }
}

fn optional_rotated_token(token: Option<String>, stored: &str) -> Result<String, CredentialError> {
    match token {
        Some(token) if !token.trim().is_empty() => Ok(token),
        Some(_) => Err(CredentialError::MalformedTokenSet),
        None => Ok(stored.to_owned()),
    }
}

fn display_metadata(id_claims: &Value) -> (Option<String>, Option<String>) {
    let email = safe_claim(id_claims.get("email"), 254);
    let plan = id_claims
        .get(AUTH_CLAIM)
        .and_then(Value::as_object)
        .and_then(|claim| claim.get("chatgpt_plan_type"))
        .and_then(|value| safe_claim(Some(value), 64));
    (email, plan)
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

    fn stored_record(
        schema_version: u8,
        access_token: &str,
        account_id: &str,
        expiry: Option<u64>,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema_version": schema_version,
            "access_token": access_token,
            "refresh_token": "refresh-token",
            "id_token": jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": account_id}})),
            "chatgpt_account_id": account_id,
            "access_token_expires_at": expiry,
            "email": null,
            "plan": null
        }))
        .unwrap()
    }

    #[test]
    fn stored_version_one_credential_is_validated_for_request_use() {
        let access_token = jwt(json!({
            AUTH_CLAIM: {"chatgpt_account_id": "account-one"},
            "exp": 2_000
        }));
        let serialized = stored_record(1, &access_token, "account-one", Some(2_000));
        let record = CredentialRecord::deserialize(&serialized, 1_000).unwrap();

        assert_eq!(
            record.into_authorization(),
            (access_token, "account-one".to_owned())
        );
    }

    #[test]
    fn status_read_keeps_its_pre_increment_contract() {
        let valid = stored_record(
            1,
            &jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}})),
            "account-one",
            Some(2_000),
        );
        let mut record: Value = serde_json::from_slice(&valid).unwrap();
        record["refresh_token"] = json!("");
        let serialized = serde_json::to_vec(&record).unwrap();

        assert!(CredentialRecord::deserialize(&serialized, 1_000).is_ok());
        assert_eq!(
            CredentialRecord::deserialize_structural(&serialized).err(),
            Some(StoredCredentialError::Malformed)
        );
    }

    #[test]
    fn an_opaque_access_token_login_can_write_is_accepted_by_both_read_paths() {
        let serialized = stored_record(1, "opaque-header-safe-token", "account-one", Some(2_000));

        assert!(CredentialRecord::deserialize(&serialized, 1_000).is_ok());
        assert!(CredentialRecord::deserialize_structural(&serialized).is_ok());
    }

    #[test]
    fn stored_credential_rejects_bad_schema_fields_and_expiry() {
        let cases = [
            (
                stored_record(
                    2,
                    &jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}})),
                    "account-one",
                    Some(2_000),
                ),
                StoredCredentialError::UnsupportedVersion,
            ),
            (b"not-json".to_vec(), StoredCredentialError::Malformed),
            (
                stored_record(1, "", "account-one", Some(2_000)),
                StoredCredentialError::Malformed,
            ),
            (
                stored_record(
                    1,
                    &jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}})),
                    "",
                    Some(2_000),
                ),
                StoredCredentialError::Malformed,
            ),
            (
                stored_record(1, "access\ntoken", "account-one", Some(2_000)),
                StoredCredentialError::Malformed,
            ),
            (
                stored_record(
                    1,
                    &jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}})),
                    "account\none",
                    Some(2_000),
                ),
                StoredCredentialError::Malformed,
            ),
            (
                stored_record(
                    1,
                    &jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}})),
                    "account-one",
                    Some(1_000),
                ),
                StoredCredentialError::Expired,
            ),
        ];

        for (serialized, expected) in cases {
            assert_eq!(
                CredentialRecord::deserialize(&serialized, 1_000).err(),
                Some(expected)
            );
        }
    }

    #[test]
    fn login_uses_the_earlier_determinable_expiry() {
        let response_deadline_first = response(
            json!({AUTH_CLAIM: {"chatgpt_account_id": "one"}}),
            json!({"exp": 9_000}),
        );
        assert_eq!(
            CredentialRecord::validate(response_deadline_first, 1_000)
                .unwrap()
                .access_token_expires_at,
            Some(4_600)
        );

        let token_deadline_first = TokenResponse {
            expires_in: Some(8_000),
            ..response(
                json!({AUTH_CLAIM: {"chatgpt_account_id": "one"}}),
                json!({"exp": 5_000}),
            )
        };
        assert_eq!(
            CredentialRecord::validate(token_deadline_first, 1_000)
                .unwrap()
                .access_token_expires_at,
            Some(5_000)
        );
    }

    #[test]
    fn login_accepts_an_opaque_header_safe_access_token() {
        let response = TokenResponse {
            access_token: "opaque-header-safe-token".to_owned(),
            refresh_token: "refresh-token".to_owned(),
            id_token: jwt(json!({
                AUTH_CLAIM: {"chatgpt_account_id": "account-one"}
            })),
            expires_in: Some(3_600),
        };

        let record = CredentialRecord::validate(response, 1_000).unwrap();

        assert_eq!(record.access_token_expires_at, Some(4_600));
        assert_eq!(
            record.into_authorization(),
            (
                "opaque-header-safe-token".to_owned(),
                "account-one".to_owned()
            )
        );
    }

    #[test]
    fn refresh_due_boundaries_are_exact_and_overflow_safe() {
        let record = |expiry| CredentialRecord {
            schema_version: SCHEMA_VERSION,
            access_token: jwt(json!({"exp": expiry})),
            refresh_token: "refresh".to_owned(),
            id_token: jwt(json!({})),
            chatgpt_account_id: "account".to_owned(),
            access_token_expires_at: expiry,
            email: None,
            plan: None,
        };

        assert!(record(None).refresh_due(1_000));
        assert!(record(Some(999)).refresh_due(1_000));
        assert!(record(Some(1_300)).refresh_due(1_000));
        assert!(!record(Some(1_301)).refresh_due(1_000));
        assert!(record(Some(u64::MAX)).refresh_due(u64::MAX - 100));
    }

    #[test]
    fn renewable_structural_loading_accepts_expired_records_but_status_does_not() {
        let access_token = jwt(json!({
            AUTH_CLAIM: {"chatgpt_account_id": "account-one"},
            "exp": 900
        }));
        let serialized = stored_record(1, &access_token, "account-one", Some(900));

        assert!(CredentialRecord::deserialize_structural(&serialized).is_ok());
        assert_eq!(
            CredentialRecord::deserialize(&serialized, 1_000).err(),
            Some(StoredCredentialError::Expired)
        );
    }

    #[test]
    fn structural_loading_requires_tokens_jwt_payloads_and_consistent_accounts() {
        let valid_access = jwt(json!({AUTH_CLAIM: {"chatgpt_account_id": "account-one"}}));
        let valid = stored_record(1, &valid_access, "account-one", Some(2_000));
        let mut cases: Vec<Value> = Vec::new();
        for (field, value) in [
            ("refresh_token", json!("")),
            ("id_token", json!("not-a-jwt")),
        ] {
            let mut record: Value = serde_json::from_slice(&valid).unwrap();
            record[field] = value;
            cases.push(record);
        }
        let mut mismatched: Value = serde_json::from_slice(&valid).unwrap();
        mismatched["id_token"] = json!(jwt(
            json!({AUTH_CLAIM: {"chatgpt_account_id": "account-two"}})
        ));
        cases.push(mismatched);
        let mut mismatched_access: Value = serde_json::from_slice(&valid).unwrap();
        mismatched_access["access_token"] = json!(jwt(
            json!({AUTH_CLAIM: {"chatgpt_account_id": "account-two"}})
        ));
        cases.push(mismatched_access);

        for record in cases {
            assert_eq!(
                CredentialRecord::deserialize_structural(&serde_json::to_vec(&record).unwrap())
                    .err(),
                Some(StoredCredentialError::Malformed)
            );
        }
    }

    fn refresh_response(account: Option<&str>, expires_in: Option<i64>) -> RefreshResponse {
        let mut claims = json!({"exp": 5_000});
        if let Some(account) = account {
            claims[AUTH_CLAIM] = json!({"chatgpt_account_id": account});
        }
        RefreshResponse {
            access_token: Some(jwt(claims)),
            refresh_token: None,
            id_token: None,
            expires_in,
        }
    }

    fn existing_record() -> CredentialRecord {
        CredentialRecord {
            schema_version: SCHEMA_VERSION,
            access_token: jwt(json!({
                AUTH_CLAIM: {"chatgpt_account_id": "account-one"},
                "exp": 2_000
            })),
            refresh_token: "old-refresh".to_owned(),
            id_token: jwt(json!({
                AUTH_CLAIM: {
                    "chatgpt_account_id": "account-one",
                    "chatgpt_plan_type": "plus"
                },
                "email": "old@example.com"
            })),
            chatgpt_account_id: "account-one".to_owned(),
            access_token_expires_at: Some(2_000),
            email: Some("old@example.com".to_owned()),
            plan: Some("plus".to_owned()),
        }
    }

    #[test]
    fn refresh_overlay_rotates_present_tokens_and_retains_omitted_fields() {
        let stored = existing_record();
        let omitted = stored
            .apply_refresh(refresh_response(None, Some(3_600)), 1_000)
            .unwrap();
        assert_eq!(omitted.refresh_token, "old-refresh");
        assert_eq!(omitted.id_token, stored.id_token);
        assert_eq!(omitted.email(), Some("old@example.com"));
        assert_eq!(omitted.plan(), Some("plus"));
        assert_eq!(omitted.chatgpt_account_id, "account-one");
        assert_eq!(omitted.access_token_expires_at, Some(4_600));

        let new_id = jwt(json!({
            AUTH_CLAIM: {"chatgpt_account_id": "account-one", "chatgpt_plan_type": "pro"},
            "email": "new@example.com"
        }));
        let rotated = stored
            .apply_refresh(
                RefreshResponse {
                    refresh_token: Some("new-refresh".to_owned()),
                    id_token: Some(new_id.clone()),
                    ..refresh_response(Some("account-one"), Some(8_000))
                },
                1_000,
            )
            .unwrap();
        assert_eq!(rotated.refresh_token, "new-refresh");
        assert_eq!(rotated.id_token, new_id);
        assert_eq!(rotated.email(), Some("new@example.com"));
        assert_eq!(rotated.plan(), Some("pro"));
        assert_eq!(rotated.access_token_expires_at, Some(5_000));
    }

    #[test]
    fn refresh_overlay_rejects_empty_mismatched_or_indeterminate_candidates() {
        let stored = existing_record();
        let cases = [
            RefreshResponse {
                access_token: None,
                ..refresh_response(None, Some(3_600))
            },
            RefreshResponse {
                refresh_token: Some(String::new()),
                ..refresh_response(None, Some(3_600))
            },
            RefreshResponse {
                id_token: Some(String::new()),
                ..refresh_response(None, Some(3_600))
            },
            RefreshResponse {
                access_token: Some(format!(
                    "{} ",
                    jwt(json!({"exp": 5_000, AUTH_CLAIM: {"chatgpt_account_id": "account-one"}}))
                )),
                ..refresh_response(None, Some(3_600))
            },
            refresh_response(Some("account-two"), Some(3_600)),
            RefreshResponse {
                access_token: Some(jwt(json!({}))),
                expires_in: None,
                refresh_token: None,
                id_token: None,
            },
        ];

        for candidate in cases {
            assert!(stored.apply_refresh(candidate, 1_000).is_err());
        }
    }

    #[test]
    fn determinate_short_lived_refresh_is_complete_and_accepted() {
        let stored = existing_record();
        let response = RefreshResponse {
            access_token: Some(jwt(json!({"exp": 1_000}))),
            refresh_token: Some("rotated-short-refresh".to_owned()),
            id_token: None,
            expires_in: Some(300),
        };

        let replacement = stored.apply_refresh(response, 1_000).unwrap();

        assert_eq!(replacement.access_token_expires_at, Some(1_000));
        assert_eq!(replacement.refresh_token, "rotated-short-refresh");
        assert!(replacement.refresh_due(1_000));
    }
}
