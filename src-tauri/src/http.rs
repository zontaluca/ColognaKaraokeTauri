//! Process-wide HTTP client. Building a `reqwest::Client` sets up a connection
//! pool and TLS configuration, so every request shares this one instead of
//! constructing a new client each time (and keeps connections to the same
//! host alive between requests).

use std::fmt::Write;

use once_cell::sync::Lazy;

pub const USER_AGENT: &str = "ColognaKaraoke/0.1 (local app)";

static CLIENT: Lazy<Option<reqwest::Client>> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| eprintln!("[http] client init failed: {}", e))
        .ok()
});

/// Shared client, or `None` if the TLS backend could not be initialised.
pub fn client() -> Option<&'static reqwest::Client> {
    CLIENT.as_ref()
}

/// Percent-encode every byte outside the RFC 3986 unreserved set.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn encodes_reserved_and_utf8_bytes() {
        assert_eq!(urlencode("a b-c_d.e~f"), "a%20b-c_d.e~f");
        assert_eq!(urlencode("è&"), "%C3%A8%26");
    }
}
