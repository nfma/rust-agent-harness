use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::form_urlencoded;

const CALLBACK_PATH: &str = "/auth/callback";
const ACCEPTED_STATE_SUFFIX: &str = ".onboarding_entrypoint=life_sciences";
const MAX_REQUEST_TARGET_BYTES: usize = 8 * 1024;
const RECEIPT_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Authorization received</title></head><body><h1>Authorization received</h1><p>Return to the terminal to finish connecting OpenAI Codex.</p></body></html>";

pub(crate) struct CallbackListener {
    server: Server,
    port: u16,
}

pub(crate) enum CallbackOutcome {
    Code(String),
    Denied,
    AuthorizationTemporarilyUnavailable,
    AuthorizationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackError {
    PortsUnavailable,
    TimedOut,
    ListenerFailed,
}

impl CallbackListener {
    pub(crate) fn bind(ports: &[u16]) -> Result<Self, CallbackError> {
        for port in ports {
            if let Ok(server) = Server::http((Ipv4Addr::LOCALHOST, *port)) {
                let bound = server
                    .server_addr()
                    .to_ip()
                    .filter(|address| address.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST))
                    .ok_or(CallbackError::ListenerFailed)?;
                return Ok(Self {
                    server,
                    port: bound.port(),
                });
            }
        }
        Err(CallbackError::PortsUnavailable)
    }

    pub(crate) fn redirect_uri(&self) -> String {
        format!("http://localhost:{}/auth/callback", self.port)
    }

    #[cfg(test)]
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn wait(
        self,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<CallbackOutcome, CallbackError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(CallbackError::TimedOut);
            }

            let Some(request) = self
                .server
                .recv_timeout(remaining.min(Duration::from_millis(100)))
                .map_err(|_| CallbackError::ListenerFailed)?
            else {
                continue;
            };

            // Preserve a validated terminal outcome even if its browser receipt cannot be delivered.
            match validate_request(&request, expected_state) {
                RequestOutcome::Code(code) => {
                    let _ = respond(request, StatusCode(200), RECEIPT_PAGE);
                    return Ok(CallbackOutcome::Code(code));
                }
                RequestOutcome::Denied => {
                    let _ = respond(
                        request,
                        StatusCode(200),
                        "Authorization was denied. Return to the terminal.",
                    );
                    return Ok(CallbackOutcome::Denied);
                }
                RequestOutcome::AuthorizationTemporarilyUnavailable => {
                    let _ = respond(
                        request,
                        StatusCode(200),
                        "Authorization did not complete. Return to the terminal.",
                    );
                    return Ok(CallbackOutcome::AuthorizationTemporarilyUnavailable);
                }
                RequestOutcome::AuthorizationFailed => {
                    let _ = respond(
                        request,
                        StatusCode(200),
                        "Authorization did not complete. Return to the terminal.",
                    );
                    return Ok(CallbackOutcome::AuthorizationFailed);
                }
                RequestOutcome::Invalid => {
                    let _ = respond(request, StatusCode(400), "Invalid authorization callback.");
                }
                RequestOutcome::Oversized => {
                    let _ = respond(request, StatusCode(414), "Request target is too large.");
                }
            }
        }
    }
}

enum RequestOutcome {
    Code(String),
    Denied,
    AuthorizationTemporarilyUnavailable,
    AuthorizationFailed,
    Invalid,
    Oversized,
}

fn validate_request(request: &Request, expected_state: &str) -> RequestOutcome {
    if request.method() != &Method::Get {
        return RequestOutcome::Invalid;
    }

    let target = request.url();
    if target.len() > MAX_REQUEST_TARGET_BYTES {
        return RequestOutcome::Oversized;
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return RequestOutcome::Invalid;
    }
    if !query_is_well_formed(query) {
        return RequestOutcome::Invalid;
    }

    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        let destination = match name.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        if destination.replace(value.into_owned()).is_some() {
            return RequestOutcome::Invalid;
        }
    }

    let Some(state) = state.filter(|value| !value.is_empty()) else {
        return RequestOutcome::Invalid;
    };
    if !accepted_state(&state, expected_state) {
        return RequestOutcome::Invalid;
    }

    match (code, error) {
        (Some(code), None) if is_safe_callback_value(&code) => RequestOutcome::Code(code),
        (None, Some(error)) if is_safe_callback_value(&error) => match error.as_str() {
            "access_denied" => RequestOutcome::Denied,
            "server_error" | "temporarily_unavailable" => {
                RequestOutcome::AuthorizationTemporarilyUnavailable
            }
            _ => RequestOutcome::AuthorizationFailed,
        },
        _ => RequestOutcome::Invalid,
    }
}

fn query_is_well_formed(query: &str) -> bool {
    let bytes = query.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let Some(high) = bytes.get(index + 1).and_then(|byte| hex_value(*byte)) else {
                    return false;
                };
                let Some(low) = bytes.get(index + 2).and_then(|byte| hex_value(*byte)) else {
                    return false;
                };
                decoded.push((high << 4) | low);
                index += 3;
            }
            byte if byte.is_ascii() => {
                decoded.push(byte);
                index += 1;
            }
            _ => return false,
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_safe_callback_value(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

fn accepted_state(candidate: &str, expected: &str) -> bool {
    candidate == expected
        || candidate
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix == ACCEPTED_STATE_SUFFIX)
}

fn respond(request: Request, status: StatusCode, body: &'static str) -> Result<(), CallbackError> {
    let content_type = Header::from_bytes("Content-Type", "text/html; charset=utf-8")
        .expect("static content type must be valid");
    request
        .respond(
            Response::from_string(body)
                .with_status_code(status)
                .with_header(content_type),
        )
        .map_err(|_| CallbackError::ListenerFailed)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;

    use super::*;

    fn request(port: u16, method: &str, target: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn run_callback(requests: Vec<(&'static str, String)>) -> (CallbackOutcome, Vec<String>) {
        let listener = CallbackListener::bind(&[0]).unwrap();
        let port = listener.port();
        let client = thread::spawn(move || {
            requests
                .into_iter()
                .map(|(method, target)| request(port, method, &target))
                .collect::<Vec<_>>()
        });
        let outcome = listener
            .wait("expected-state", Duration::from_secs(2))
            .unwrap();
        (outcome, client.join().unwrap())
    }

    #[test]
    fn valid_callback_returns_code_and_static_receipt() {
        let listener = CallbackListener::bind(&[0]).unwrap();
        let port = listener.port();
        let client = thread::spawn(move || {
            request(
                port,
                "GET",
                "/auth/callback?code=secret-code&state=expected-state",
            )
        });

        let outcome = listener
            .wait("expected-state", Duration::from_secs(2))
            .unwrap();
        assert!(matches!(outcome, CallbackOutcome::Code(code) if code == "secret-code"));
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Authorization received"));
        assert!(!response.contains("secret-code"));
        assert!(!response.contains("expected-state"));
    }

    #[test]
    fn invalid_requests_do_not_consume_the_valid_callback() {
        let oversized = format!("/auth/callback?{}", "x".repeat(MAX_REQUEST_TARGET_BYTES));
        let (outcome, responses) = run_callback(vec![
            ("GET", "/wrong?code=x&state=expected-state".to_owned()),
            (
                "POST",
                "/auth/callback?code=x&state=expected-state".to_owned(),
            ),
            ("GET", "/auth/callback?code=x&state=wrong-state".to_owned()),
            ("GET", "/auth/callback?state=expected-state".to_owned()),
            (
                "GET",
                "/auth/callback?code=&state=expected-state".to_owned(),
            ),
            ("GET", "/auth/callback?code=x".to_owned()),
            (
                "GET",
                "/auth/callback?code=%GG&state=expected-state".to_owned(),
            ),
            (
                "GET",
                "/auth/callback?code=%FF&state=expected-state".to_owned(),
            ),
            ("GET", oversized),
            (
                "GET",
                "/auth/callback?code=secret-code&state=expected-state".to_owned(),
            ),
        ]);

        assert!(matches!(outcome, CallbackOutcome::Code(code) if code == "secret-code"));
        assert!(
            responses[..8]
                .iter()
                .all(|response| response.starts_with("HTTP/1.1 400"))
        );
        assert!(responses[8].starts_with("HTTP/1.1 414"));
        assert!(responses[9].starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn accepts_only_the_pinned_state_suffix() {
        let (outcome, responses) = run_callback(vec![
            (
                "GET",
                "/auth/callback?code=x&state=expected-state.unknown=true".to_owned(),
            ),
            (
                "GET",
                format!("/auth/callback?code=accepted&state=expected-state{ACCEPTED_STATE_SUFFIX}"),
            ),
        ]);

        assert!(matches!(outcome, CallbackOutcome::Code(code) if code == "accepted"));
        assert!(responses[0].starts_with("HTTP/1.1 400"));
        assert!(responses[1].starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn denial_with_expected_state_is_terminal() {
        let (outcome, responses) = run_callback(vec![(
            "GET",
            "/auth/callback?error=access_denied&state=expected-state".to_owned(),
        )]);

        assert!(matches!(outcome, CallbackOutcome::Denied));
        assert!(responses[0].starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn provider_errors_are_categorized_without_echoing_codes() {
        for (error, temporarily_unavailable) in [
            ("server_error", true),
            ("temporarily_unavailable", true),
            ("invalid_request", false),
        ] {
            let (outcome, responses) = run_callback(vec![(
                "GET",
                format!("/auth/callback?error={error}&state=expected-state"),
            )]);

            if temporarily_unavailable {
                assert!(matches!(
                    outcome,
                    CallbackOutcome::AuthorizationTemporarilyUnavailable
                ));
            } else {
                assert!(matches!(outcome, CallbackOutcome::AuthorizationFailed));
            }
            assert!(responses[0].starts_with("HTTP/1.1 200"));
            assert!(!responses[0].contains(error));
        }
    }

    #[test]
    fn timeout_is_bounded() {
        let listener = CallbackListener::bind(&[0]).unwrap();
        let result = listener.wait("expected-state", Duration::from_millis(20));

        assert_eq!(result.err(), Some(CallbackError::TimedOut));
    }

    #[test]
    fn tries_ports_in_order_and_never_contacts_an_incumbent() {
        let incumbent = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        incumbent.set_nonblocking(true).unwrap();
        let occupied = incumbent.local_addr().unwrap().port();
        let listener = CallbackListener::bind(&[occupied, 0]).unwrap();

        assert_ne!(listener.port(), occupied);
        assert!(matches!(
            incumbent.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn fails_when_every_port_is_occupied() {
        let first = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let ports = [
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
        ];

        assert!(matches!(
            CallbackListener::bind(&ports),
            Err(CallbackError::PortsUnavailable)
        ));
    }
}
