use std::io::Read;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;
use url::Url;

use crate::pkce::AuthSecrets;

pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const ORIGINATOR: &str = "rust-agent-harness";
pub(crate) const SCOPES: &str = "openid profile email offline_access";

pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) id_token: String,
    pub(crate) expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct WireTokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: String,
    expires_in: Option<i64>,
}

#[derive(Clone)]
pub(crate) struct ExchangeConfig {
    pub(crate) token_endpoint: Url,
    pub(crate) connect_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) response_body_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExchangeError {
    Unavailable,
    Rejected,
    ResponseTooLarge,
    MalformedResponse,
}

pub(crate) fn authorization_url(endpoint: &Url, redirect_uri: &str, secrets: &AuthSecrets) -> Url {
    let mut url = endpoint.clone();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &secrets.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &secrets.state)
        .append_pair("originator", ORIGINATOR)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true");
    url
}

pub(crate) fn exchange_code(
    config: &ExchangeConfig,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<TokenResponse, ExchangeError> {
    let client = Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ExchangeError::Unavailable)?;

    let mut response = client
        .post(config.token_endpoint.clone())
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ])
        .send()
        .map_err(|_| ExchangeError::Unavailable)?;

    let status = response.status();
    if !status.is_success() {
        return if status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
        {
            Err(ExchangeError::Unavailable)
        } else {
            Err(ExchangeError::Rejected)
        };
    }

    if response
        .content_length()
        .is_some_and(|length| length > config.response_body_limit as u64)
    {
        return Err(ExchangeError::ResponseTooLarge);
    }

    let mut body = Vec::new();
    response
        .by_ref()
        .take(config.response_body_limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| ExchangeError::Unavailable)?;
    if body.len() > config.response_body_limit {
        return Err(ExchangeError::ResponseTooLarge);
    }

    let response: WireTokenResponse =
        serde_json::from_slice(&body).map_err(|_| ExchangeError::MalformedResponse)?;
    Ok(TokenResponse {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        id_token: response.id_token,
        expires_in: response.expires_in,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{Ipv4Addr, TcpListener};
    use std::thread;

    use tiny_http::{Header, Response, Server};

    use super::*;

    #[test]
    fn authorization_parameters_match_the_pinned_contract() {
        let endpoint = Url::parse("https://auth.openai.com/oauth/authorize").unwrap();
        let secrets = AuthSecrets::fixed("state-value", "verifier-value");
        for port in [1455, 1457] {
            let redirect_uri = format!("http://localhost:{port}/auth/callback");
            let url = authorization_url(&endpoint, &redirect_uri, &secrets);
            let parameters: HashMap<_, _> = url.query_pairs().into_owned().collect();

            assert_eq!(url.scheme(), "https");
            assert_eq!(url.host_str(), Some("auth.openai.com"));
            assert_eq!(url.path(), "/oauth/authorize");
            assert_eq!(
                parameters.get("response_type").map(String::as_str),
                Some("code")
            );
            assert_eq!(
                parameters.get("client_id").map(String::as_str),
                Some(CLIENT_ID)
            );
            assert_eq!(
                parameters.get("redirect_uri").map(String::as_str),
                Some(redirect_uri.as_str())
            );
            assert_eq!(parameters.get("scope").map(String::as_str), Some(SCOPES));
            assert!(!parameters["scope"].contains("connector"));
            assert_eq!(
                parameters.get("code_challenge_method").map(String::as_str),
                Some("S256")
            );
            assert_eq!(
                parameters.get("state").map(String::as_str),
                Some("state-value")
            );
            assert_eq!(
                parameters.get("originator").map(String::as_str),
                Some(ORIGINATOR)
            );
            assert_eq!(
                parameters
                    .get("id_token_add_organizations")
                    .map(String::as_str),
                Some("true")
            );
            assert_eq!(
                parameters
                    .get("codex_cli_simplified_flow")
                    .map(String::as_str),
                Some("true")
            );
        }
    }

    fn exchange_config(endpoint: Url, response_body_limit: usize) -> ExchangeConfig {
        ExchangeConfig {
            token_endpoint: endpoint,
            connect_timeout: Duration::from_millis(50),
            request_timeout: Duration::from_millis(50),
            response_body_limit,
        }
    }

    fn one_response_server(status: u16, body: Vec<u8>) -> (Url, thread::JoinHandle<()>) {
        let server = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let thread = thread::spawn(move || {
            let request = server.recv().unwrap();
            request
                .respond(Response::from_data(body).with_status_code(status))
                .unwrap();
        });
        (
            Url::parse(&format!("http://127.0.0.1:{port}/oauth/token")).unwrap(),
            thread,
        )
    }

    #[test]
    fn exchange_classifies_non_success_and_rejects_malformed_or_oversized_responses() {
        let cases = [
            (
                400,
                b"bad request body".to_vec(),
                64 * 1024,
                ExchangeError::Rejected,
            ),
            (
                403,
                b"forbidden body".to_vec(),
                64 * 1024,
                ExchangeError::Rejected,
            ),
            (
                408,
                b"request timeout body".to_vec(),
                64 * 1024,
                ExchangeError::Unavailable,
            ),
            (
                429,
                b"rate limit body".to_vec(),
                64 * 1024,
                ExchangeError::Unavailable,
            ),
            (
                500,
                b"server error body".to_vec(),
                64 * 1024,
                ExchangeError::Unavailable,
            ),
            (
                599,
                b"provider unavailable body".to_vec(),
                64 * 1024,
                ExchangeError::Unavailable,
            ),
            (
                200,
                b"not-json".to_vec(),
                64 * 1024,
                ExchangeError::MalformedResponse,
            ),
            (
                200,
                br#"{"access_token":"access"}"#.to_vec(),
                64 * 1024,
                ExchangeError::MalformedResponse,
            ),
            (200, vec![b'x'; 65], 64, ExchangeError::ResponseTooLarge),
        ];

        for (status, body, limit, expected) in cases {
            let (endpoint, server) = one_response_server(status, body);
            let result = exchange_code(
                &exchange_config(endpoint, limit),
                "code",
                "http://localhost:1455/auth/callback",
                "verifier",
            );
            server.join().unwrap();
            assert_eq!(result.err(), Some(expected));
        }
    }

    #[test]
    fn exchange_rejects_redirect_without_contacting_the_second_origin() {
        let issuer = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let issuer_port = issuer.server_addr().to_ip().unwrap().port();
        let second_origin = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second_origin_port = second_origin.local_addr().unwrap().port();
        let redirect_url = format!("http://127.0.0.1:{second_origin_port}/oauth/token");
        let issuer_thread = thread::spawn(move || {
            let request = issuer.recv().unwrap();
            let location = Header::from_bytes("Location", redirect_url).unwrap();
            request
                .respond(Response::empty(307).with_header(location))
                .unwrap();
        });

        let result = exchange_code(
            &exchange_config(
                Url::parse(&format!("http://127.0.0.1:{issuer_port}/oauth/token")).unwrap(),
                1024,
            ),
            "code",
            "http://localhost:1455/auth/callback",
            "verifier",
        );
        issuer_thread.join().unwrap();
        second_origin.set_nonblocking(true).unwrap();

        assert!(matches!(
            second_origin.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert_eq!(result.err(), Some(ExchangeError::Rejected));
    }

    #[test]
    fn exchange_connect_and_request_failures_are_bounded() {
        let unused_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let unused_port = unused_listener.local_addr().unwrap().port();
        drop(unused_listener);
        let unavailable = exchange_code(
            &exchange_config(
                Url::parse(&format!("http://127.0.0.1:{unused_port}/oauth/token")).unwrap(),
                1024,
            ),
            "code",
            "http://localhost:1455/auth/callback",
            "verifier",
        );
        assert_eq!(unavailable.err(), Some(ExchangeError::Unavailable));

        let stalled = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let stalled_port = stalled.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (_connection, _) = stalled.accept().unwrap();
            thread::sleep(Duration::from_millis(100));
        });
        let timed_out = exchange_code(
            &exchange_config(
                Url::parse(&format!("http://127.0.0.1:{stalled_port}/oauth/token")).unwrap(),
                1024,
            ),
            "code",
            "http://localhost:1455/auth/callback",
            "verifier",
        );
        server.join().unwrap();
        assert_eq!(timed_out.err(), Some(ExchangeError::Unavailable));
    }
}
