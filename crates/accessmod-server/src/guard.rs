//! Who may reach a server on this machine.
//!
//! A server on a loopback address answers only requests naming a loopback
//! host: a web page cannot reach it through a DNS name resolving to
//! 127.0.0.1 (DNS rebinding). The desktop application also gives its server
//! a session token: the window opens [`OPEN_PATH`] with it once, the server
//! answers with a `SameSite=Strict` cookie, and every other request needs
//! that cookie. Other users of the machine, and pages in a browser, cannot
//! use the server.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

/// Opens a session: `GET /api/session/open?token=…`, then redirects to `/`.
pub const OPEN_PATH: &str = "/api/session/open";

const COOKIE: &str = "accessmod_session";

/// Checks applied before any route.
#[derive(Debug, Clone, Default)]
pub struct Guard {
    /// Answer only requests whose `Host` is a loopback name or address.
    pub loopback_host: bool,
    /// Require this session token (desktop application).
    pub token: Option<String>,
}

impl Guard {
    /// Whether the guard checks anything.
    pub fn is_open(&self) -> bool {
        !self.loopback_host && self.token.is_none()
    }

    /// `router` behind the guard.
    pub fn wrap(self, router: Router) -> Router {
        if self.is_open() {
            return router;
        }
        router.layer(middleware::from_fn_with_state(Arc::new(self), check))
    }
}

async fn check(State(guard): State<Arc<Guard>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if guard.loopback_host && !host_is_loopback(headers) {
        return forbidden("this server answers only on 127.0.0.1 or localhost");
    }
    let Some(token) = &guard.token else {
        return next.run(request).await;
    };
    if request.uri().path() == OPEN_PATH {
        let given = query_value(request.uri().query().unwrap_or(""), "token");
        if !given.is_some_and(|given| same(given, token)) {
            return forbidden("invalid session token");
        }
        let cookie = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict");
        return (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, "/".to_string()),
                (header::SET_COOKIE, cookie),
            ],
        )
            .into_response();
    }
    if !cookie_value(headers, COOKIE).is_some_and(|given| same(given, token)) {
        return forbidden("this server belongs to an AccessMod window");
    }
    next.run(request).await
}

fn forbidden(message: &str) -> Response {
    ApiError::Forbidden(message.to_string()).into_response()
}

/// Whether the `Host` header names this machine: `localhost` or a loopback
/// address, any port.
fn host_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let name = match host.strip_prefix('[') {
        // [::1]:5099
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

/// Compare without stopping at the first difference (timing).
fn same(given: &str, token: &str) -> bool {
    given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn host(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn loopback_hosts() {
        for ok in [
            "127.0.0.1:5099",
            "localhost",
            "LOCALHOST:1",
            "[::1]:80",
            "127.0.0.2",
        ] {
            assert!(host_is_loopback(&host(ok)), "{ok}");
        }
        for bad in [
            "evil.example:5099",
            "192.168.1.2",
            "[::2]:80",
            "localhost.evil",
        ] {
            assert!(!host_is_loopback(&host(bad)), "{bad}");
        }
        assert!(!host_is_loopback(&HeaderMap::new()));
    }

    #[test]
    fn tokens_and_cookies() {
        assert!(same("abc", "abc"));
        assert!(!same("abd", "abc"));
        assert!(!same("ab", "abc"));
        assert_eq!(query_value("a=1&token=xyz", "token"), Some("xyz"));
        assert_eq!(query_value("tokens=1", "token"), None);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("x=1; accessmod_session=s3; y=2"),
        );
        assert_eq!(cookie_value(&headers, COOKIE), Some("s3"));
    }
}
