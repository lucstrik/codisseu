//! No shell/file/custom URL capability crosses the browser address boundary.
use url::{Host, Url};
pub const MAX_URL: usize = 8192;
pub fn validate_url(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > MAX_URL || value.chars().any(char::is_control) {
        return Err("Endereço vazio, excessivo ou com caracteres de controle".into());
    }
    if value == "about:blank" {
        return Ok(value.into());
    }
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default());
    if authority.is_some_and(|s| s.contains('@')) {
        return Err("Credenciais em endereço não são permitidas".into());
    }
    let url = Url::parse(value).map_err(|_| "Endereço HTTP/HTTPS inválido")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use HTTP/HTTPS sem credenciais, ou about:blank".into());
    }
    let canonical = url.to_string();
    if canonical.len() > MAX_URL {
        return Err("Endereço normalizado excessivo".into());
    }
    Ok(canonical)
}
pub fn normalize_url(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("Endereço contém caracteres de controle".into());
    }
    let value = value.trim();
    if value.contains("://") || value == "about:blank" {
        return validate_url(value);
    }
    // A loopback host with a numeric port is an address, not a custom scheme.
    // Reject every other explicit scheme before applying the default HTTPS.
    let probe = Url::parse(&format!("http://{value}")).map_err(|_| "Endereço inválido")?;
    let loopback = match probe.host() {
        Some(Host::Domain(n)) => {
            n.trim_end_matches('.').eq_ignore_ascii_case("localhost")
                || n.trim_end_matches('.').ends_with(".localhost")
        }
        Some(Host::Ipv4(a)) => a.is_loopback(),
        Some(Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    };
    if value.contains(':') && !value.starts_with('[') {
        let before_path = value.split(['/', '?', '#']).next().unwrap_or_default();
        if let Some((_, port)) = before_path.rsplit_once(':')
            && !port.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("Protocolo de endereço não permitido".into());
        }
        // `javascript:123` must not turn into an HTTPS host and numeric port.
        if Url::parse(value).is_ok() && !loopback && !before_path.contains('.') {
            return Err("Protocolo de endereço não permitido".into());
        }
    }
    validate_url(&format!(
        "{}://{value}",
        if loopback { "http" } else { "https" }
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_are_canonical_network_only_and_loopback_is_local_http() {
        for (input, expected) in [
            ("localhost:5173/path", "http://localhost:5173/path"),
            ("127.0.0.1:8080", "http://127.0.0.1:8080/"),
            ("[::1]:3000", "http://[::1]:3000/"),
            ("example.com", "https://example.com/"),
            (
                "https://exemplo.com/ação",
                "https://exemplo.com/a%C3%A7%C3%A3o",
            ),
            ("about:blank", "about:blank"),
        ] {
            assert_eq!(normalize_url(input).unwrap(), expected);
        }
        for input in [
            "file:///tmp/source",
            "javascript:alert(1)",
            "javascript:123",
            "data:text/html,hello",
            "chrome://settings",
            "https://user:secret@host/",
            "https://host/\nnext",
            "about:config",
            "https://",
        ] {
            assert!(normalize_url(input).is_err(), "{input}");
        }
    }
}
