use std::fmt;
use std::io::Read;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::oauth::CLIENT_ID;

const ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const ELAPSED_BUDGET: Duration = Duration::from_secs(30);
const RESPONSE_BODY_LIMIT: usize = 64 * 1024;

pub(crate) trait RefreshClient {
    fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, RefreshError>;
}

pub(crate) struct HttpRefreshClient {
    config: RefreshConfig,
}

impl HttpRefreshClient {
    #[cfg(target_os = "macos")]
    pub(crate) fn production() -> Self {
        Self {
            config: RefreshConfig::production(),
        }
    }

    #[cfg(test)]
    fn with_config(config: RefreshConfig) -> Self {
        Self { config }
    }
}

impl RefreshClient for HttpRefreshClient {
    fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, RefreshError> {
        request_refresh(&self.config, refresh_token)
    }
}

#[derive(Clone)]
struct RefreshConfig {
    endpoint: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    elapsed_budget: Duration,
    response_body_limit: usize,
}

impl RefreshConfig {
    fn production() -> Self {
        Self {
            endpoint: Url::parse(ENDPOINT).expect("production refresh endpoint must be valid"),
            connect_timeout: CONNECT_TIMEOUT,
            request_timeout: REQUEST_TIMEOUT,
            elapsed_budget: ELAPSED_BUDGET,
            response_body_limit: RESPONSE_BODY_LIMIT,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RefreshResponse {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: Option<String>,
    pub(crate) expires_in: Option<i64>,
}

impl fmt::Debug for RefreshResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RefreshResponse([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RefreshError {
    ReauthenticationRequired,
    Unavailable,
}

#[derive(Serialize)]
struct RefreshRequest<'a> {
    grant_type: &'static str,
    refresh_token: &'a str,
    client_id: &'static str,
}

#[derive(Deserialize)]
struct WireRefreshResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<i64>,
}

fn request_refresh(
    config: &RefreshConfig,
    refresh_token: &str,
) -> Result<RefreshResponse, RefreshError> {
    let started = Instant::now();
    let client = Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .redirect(Policy::none())
        .build()
        .map_err(|_| RefreshError::Unavailable)?;
    check_budget(started, config.elapsed_budget)?;
    let body = serde_json::to_vec(&RefreshRequest {
        grant_type: "refresh_token",
        refresh_token,
        client_id: CLIENT_ID,
    })
    .map_err(|_| RefreshError::Unavailable)?;
    let response = client
        .post(config.endpoint.clone())
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .map_err(|_| RefreshError::Unavailable)?;
    check_budget(started, config.elapsed_budget)?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(RefreshError::ReauthenticationRequired);
    }
    let body = read_bounded(response, config, started)?;
    if !status.is_success() {
        return if permanent_error_code(&body) {
            Err(RefreshError::ReauthenticationRequired)
        } else {
            Err(RefreshError::Unavailable)
        };
    }

    let response: WireRefreshResponse =
        serde_json::from_slice(&body).map_err(|_| RefreshError::Unavailable)?;
    Ok(RefreshResponse {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        id_token: response.id_token,
        expires_in: response.expires_in,
    })
}

fn read_bounded(
    mut response: Response,
    config: &RefreshConfig,
    started: Instant,
) -> Result<Vec<u8>, RefreshError> {
    if response
        .content_length()
        .is_some_and(|length| length > config.response_body_limit as u64)
    {
        return Err(RefreshError::Unavailable);
    }

    let mut body = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        check_budget(started, config.elapsed_budget)?;
        let read = response
            .read(&mut buffer)
            .map_err(|_| RefreshError::Unavailable)?;
        check_budget(started, config.elapsed_budget)?;
        if read == 0 {
            break;
        }
        if body.len().saturating_add(read) > config.response_body_limit {
            return Err(RefreshError::Unavailable);
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(body)
}

fn check_budget(started: Instant, budget: Duration) -> Result<(), RefreshError> {
    if started.elapsed() >= budget {
        Err(RefreshError::Unavailable)
    } else {
        Ok(())
    }
}

fn permanent_error_code(body: &[u8]) -> bool {
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let code = match body.get("error") {
        Some(Value::Object(error)) => error.get("code").and_then(Value::as_str),
        Some(Value::String(code)) => Some(code.as_str()),
        _ => body.get("code").and_then(Value::as_str),
    };
    code.is_some_and(|code| {
        matches!(
            code.to_ascii_lowercase().as_str(),
            "refresh_token_expired" | "refresh_token_reused" | "refresh_token_invalidated"
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::json;
    use tiny_http::{Header, Response as TinyResponse, Server};

    use super::*;

    struct CapturedRequest {
        method: String,
        target: String,
        content_type: Option<String>,
        body: Vec<u8>,
    }

    fn config(endpoint: Url, body_limit: usize) -> RefreshConfig {
        RefreshConfig {
            endpoint,
            connect_timeout: Duration::from_millis(100),
            request_timeout: Duration::from_millis(100),
            elapsed_budget: Duration::from_secs(1),
            response_body_limit: body_limit,
        }
    }

    fn one_response_server(
        status: u16,
        body: Vec<u8>,
    ) -> (Url, thread::JoinHandle<CapturedRequest>) {
        let server = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let thread = thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let mut request_body = Vec::new();
            request.as_reader().read_to_end(&mut request_body).unwrap();
            let captured = CapturedRequest {
                method: request.method().as_str().to_owned(),
                target: request.url().to_owned(),
                content_type: request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("Content-Type"))
                    .map(|header| header.value.as_str().to_owned()),
                body: request_body,
            };
            request
                .respond(TinyResponse::from_data(body).with_status_code(status))
                .unwrap();
            captured
        });
        (
            Url::parse(&format!("http://127.0.0.1:{port}/oauth/token")).unwrap(),
            thread,
        )
    }

    #[test]
    fn refresh_request_and_success_shape_match_the_exact_json_contract() {
        let body = serde_json::to_vec(&json!({
            "access_token": "new-access-token",
            "refresh_token": "new-refresh-token",
            "id_token": "new-id-token",
            "expires_in": 3600
        }))
        .unwrap();
        let (endpoint, server) = one_response_server(200, body);
        let client = HttpRefreshClient::with_config(config(endpoint, RESPONSE_BODY_LIMIT));

        let response = client.refresh("old-refresh-token-sentinel").unwrap();
        let request = server.join().unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/oauth/token");
        assert!(!request.target.contains("old-refresh-token-sentinel"));
        assert_eq!(request.content_type.as_deref(), Some("application/json"));
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            json!({
                "grant_type": "refresh_token",
                "refresh_token": "old-refresh-token-sentinel",
                "client_id": "app_EMoamEEZ73f0CkXaXp7hrann"
            })
        );
        assert_eq!(
            response,
            RefreshResponse {
                access_token: Some("new-access-token".to_owned()),
                refresh_token: Some("new-refresh-token".to_owned()),
                id_token: Some("new-id-token".to_owned()),
                expires_in: Some(3_600),
            }
        );
        assert_eq!(format!("{response:?}"), "RefreshResponse([REDACTED])");
    }

    #[test]
    fn provider_errors_are_classified_without_retaining_bodies() {
        let cases = [
            (
                401,
                b"not-json with token-shaped-sentinel".to_vec(),
                RefreshError::ReauthenticationRequired,
            ),
            (
                400,
                br#"{"error":{"code":"refresh_token_expired"}}"#.to_vec(),
                RefreshError::ReauthenticationRequired,
            ),
            (
                403,
                br#"{"error":"refresh_token_reused"}"#.to_vec(),
                RefreshError::ReauthenticationRequired,
            ),
            (
                400,
                br#"{"code":"REFRESH_TOKEN_INVALIDATED"}"#.to_vec(),
                RefreshError::ReauthenticationRequired,
            ),
            (
                408,
                br#"{"error":"timeout"}"#.to_vec(),
                RefreshError::Unavailable,
            ),
            (
                429,
                br#"{"error":"limited"}"#.to_vec(),
                RefreshError::Unavailable,
            ),
            (
                500,
                br#"{"error":"server"}"#.to_vec(),
                RefreshError::Unavailable,
            ),
            (
                400,
                br#"{"error":"other"}"#.to_vec(),
                RefreshError::Unavailable,
            ),
            (
                400,
                b"malformed error body".to_vec(),
                RefreshError::Unavailable,
            ),
            (
                200,
                b"malformed success body".to_vec(),
                RefreshError::Unavailable,
            ),
        ];

        for (status, body, expected) in cases {
            let (endpoint, server) = one_response_server(status, body);
            let client = HttpRefreshClient::with_config(config(endpoint, RESPONSE_BODY_LIMIT));
            let error = client.refresh("refresh-token-sentinel").unwrap_err();
            server.join().unwrap();

            assert_eq!(error, expected);
            let rendered = format!("{error:?}");
            assert!(!rendered.contains("token-shaped-sentinel"));
            assert!(!rendered.contains("refresh-token-sentinel"));
        }
    }

    #[test]
    fn redirect_is_rejected_without_contacting_the_location() {
        let target = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target_port = target.server_addr().to_ip().unwrap().port();
        let target_contacted = Arc::new(Mutex::new(None));
        let target_result = Arc::clone(&target_contacted);
        let target_thread = thread::spawn(move || {
            *target_result.lock().unwrap() = Some(
                target
                    .recv_timeout(Duration::from_millis(250))
                    .unwrap()
                    .is_some(),
            );
        });
        let issuer = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let issuer_port = issuer.server_addr().to_ip().unwrap().port();
        let issuer_thread = thread::spawn(move || {
            let request = issuer.recv().unwrap();
            let location = Header::from_bytes(
                "Location",
                format!("http://127.0.0.1:{target_port}/must-not-be-contacted"),
            )
            .unwrap();
            request
                .respond(TinyResponse::empty(307).with_header(location))
                .unwrap();
        });
        let endpoint = Url::parse(&format!("http://127.0.0.1:{issuer_port}/oauth/token")).unwrap();
        let client = HttpRefreshClient::with_config(config(endpoint, RESPONSE_BODY_LIMIT));

        let result = client.refresh("refresh-token");
        issuer_thread.join().unwrap();
        target_thread.join().unwrap();

        assert_eq!(result, Err(RefreshError::Unavailable));
        assert_eq!(*target_contacted.lock().unwrap(), Some(false));
    }

    fn raw_server(
        response: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    return;
                }
                bytes.extend_from_slice(&buffer[..read]);
            }
            response(stream);
        });
        (
            Url::parse(&format!("http://127.0.0.1:{port}/oauth/token")).unwrap(),
            thread,
        )
    }

    #[test]
    fn response_limit_applies_to_declared_and_absent_lengths() {
        let oversized_success = serde_json::to_vec(&json!({
            "access_token": "new-access-token",
            "padding": "x".repeat(128)
        }))
        .unwrap();
        let (endpoint, server) = one_response_server(200, oversized_success.clone());
        let client = HttpRefreshClient::with_config(config(endpoint, 64));
        assert_eq!(client.refresh("refresh"), Err(RefreshError::Unavailable));
        server.join().unwrap();

        let (endpoint, server) = raw_server(move |mut stream| {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.write_all(&oversized_success).unwrap();
        });
        let client = HttpRefreshClient::with_config(config(endpoint, 64));
        assert_eq!(client.refresh("refresh"), Err(RefreshError::Unavailable));
        server.join().unwrap();
    }

    #[test]
    fn stalled_and_slow_drip_reads_obey_inactivity_and_elapsed_budgets() {
        let (endpoint, server) = raw_server(|mut stream| {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n")
                .unwrap();
            thread::sleep(Duration::from_millis(100));
        });
        let mut stalled = config(endpoint, RESPONSE_BODY_LIMIT);
        stalled.request_timeout = Duration::from_millis(20);
        let client = HttpRefreshClient::with_config(stalled);
        assert_eq!(client.refresh("refresh"), Err(RefreshError::Unavailable));
        server.join().unwrap();

        let (endpoint, server) = raw_server(|mut stream| {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                .unwrap();
            for byte in b"null" {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(15));
            }
        });
        let mut slow = config(endpoint, RESPONSE_BODY_LIMIT);
        slow.request_timeout = Duration::from_millis(100);
        slow.elapsed_budget = Duration::from_millis(25);
        let client = HttpRefreshClient::with_config(slow);
        assert_eq!(client.refresh("refresh"), Err(RefreshError::Unavailable));
        server.join().unwrap();
    }

    #[test]
    fn production_configuration_pins_endpoint_and_true_transport_bounds() {
        let config = RefreshConfig::production();

        assert_eq!(config.endpoint.as_str(), ENDPOINT);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.elapsed_budget, Duration::from_secs(30));
        assert_eq!(config.response_body_limit, 64 * 1024);
        assert_eq!(
            config.request_timeout + config.elapsed_budget,
            Duration::from_secs(60)
        );
    }
}
