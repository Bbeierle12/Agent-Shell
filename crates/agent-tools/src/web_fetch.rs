use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use url::Url;

/// Fetch a web page and return its text content.
///
/// Uses per-request DNS pinning to prevent TOCTOU / DNS-rebinding SSRF:
/// we resolve DNS once, validate every returned IP, then force `reqwest`
/// to connect to the already-validated addresses.
pub struct WebFetchTool;

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }

    /// Build a per-request client whose DNS is pinned to the pre-validated
    /// IP addresses, closing the TOCTOU window.
    ///
    /// Uses a custom redirect policy that validates each redirect target
    /// through the same SSRF checks as the initial request.
    fn build_pinned_client(domain: &str, port: u16, addrs: &[SocketAddr]) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("agent-shell/0.1")
            .redirect(ssrf_safe_redirect_policy());

        // Pin every validated address so reqwest never re-resolves.
        for addr in addrs {
            builder = builder.resolve_to_addrs(domain, &[*addr]);
        }
        // Also pin with port in case redirects try a different port.
        let host_with_port = format!("{}:{}", domain, port);
        for addr in addrs {
            builder = builder.resolve(&host_with_port, *addr);
        }

        builder.build().unwrap_or_default()
    }
}

/// Maximum response body size to buffer (2 MB). Prevents OOM from huge responses.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Custom redirect policy that validates each redirect target through SSRF checks.
///
/// Without this, a public URL could 302 to `http://127.0.0.1/...` and reqwest
/// would follow it, bypassing the initial SSRF validation.
fn ssrf_safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else {
            let url = attempt.url();
            // Validate the redirect target through the same SSRF checks.
            match validate_url_not_internal(url.as_str()) {
                Ok(_) => attempt.follow(),
                Err(e) => attempt.error(format!("redirect blocked by SSRF check: {}", e)),
            }
        }
    })
}

/// Check if an IP address is in a private/internal range.
pub(crate) fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()              // 127.0.0.0/8
            || v4.is_private()            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
            || v4.is_link_local()         // 169.254.0.0/16
            || v4.is_broadcast()          // 255.255.255.255
            || v4.is_unspecified()        // 0.0.0.0
            || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // CGN 100.64.0.0/10
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()              // ::1
            || v6.is_unspecified()        // ::
            // IPv4-mapped IPv6 (::ffff:0:0/96) — check the embedded v4
            || matches!(v6.to_ipv4_mapped(), Some(v4) if is_private_ip(&IpAddr::V4(v4)))
            // Unique local addresses (fc00::/7)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
            // Link-local (fe80::/10)
            || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Validated URL with pre-resolved addresses pinned for TOCTOU-safe fetching.
#[derive(Debug)]
pub(crate) struct ValidatedUrl {
    pub url: Url,
    /// The domain name (if the host was a domain, not a raw IP).
    pub domain: Option<String>,
    /// Port (defaults to 80/443 based on scheme).
    pub port: u16,
    /// Pre-resolved and validated socket addresses to pin in reqwest.
    pub resolved_addrs: Vec<SocketAddr>,
}

/// Validate that a URL is safe to fetch (not an internal/SSRF target).
///
/// Returns a `ValidatedUrl` with pre-resolved addresses so the caller
/// can pin DNS in `reqwest`, eliminating the TOCTOU / DNS-rebinding window.
pub(crate) fn validate_url_not_internal(raw_url: &str) -> Result<ValidatedUrl, AgentError> {
    let parsed = Url::parse(raw_url).map_err(|e| AgentError::ToolExecution {
        tool_name: "web_fetch".into(),
        message: format!("Invalid URL: {}", e),
    })?;

    // Only allow http and https schemes.
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Scheme '{}' is not allowed (only http/https)", other),
            });
        }
    }

    // Block internal hostnames.
    let host = parsed.host_str().unwrap_or("");
    let blocked_hosts = ["localhost", "metadata.google.internal"];
    let blocked_suffixes = [".local", ".internal", ".localhost"];
    let lower_host = host.to_lowercase();
    if blocked_hosts.contains(&lower_host.as_str()) {
        return Err(AgentError::ToolExecution {
            tool_name: "web_fetch".into(),
            message: format!("Host '{}' is blocked (internal address)", host),
        });
    }
    for suffix in &blocked_suffixes {
        if lower_host.ends_with(suffix) {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Host '{}' is blocked (internal address)", host),
            });
        }
    }

    let port = parsed.port_or_known_default().unwrap_or(80);

    // Resolve DNS and block private IPs.
    // Check if the host is a raw IP address first.
    if let Some(url::Host::Ipv4(ip)) = parsed.host() {
        if is_private_ip(&IpAddr::V4(ip)) {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("IP address {} is a private/internal address", ip),
            });
        }
        return Ok(ValidatedUrl {
            url: parsed,
            domain: None,
            port,
            resolved_addrs: vec![SocketAddr::new(IpAddr::V4(ip), port)],
        });
    }
    if let Some(url::Host::Ipv6(ip)) = parsed.host() {
        if is_private_ip(&IpAddr::V6(ip)) {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("IP address {} is a private/internal address", ip),
            });
        }
        return Ok(ValidatedUrl {
            url: parsed,
            domain: None,
            port,
            resolved_addrs: vec![SocketAddr::new(IpAddr::V6(ip), port)],
        });
    }

    // For domain names, perform DNS resolution and check ALL resolved IPs.
    let mut safe_addrs = Vec::new();
    if let Some(url::Host::Domain(domain)) = parsed.host() {
        let domain_owned = domain.to_string();
        let addr_str = format!("{}:{}", domain_owned, port);
        let addrs: Vec<SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&addr_str)
            .map_err(|e| AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("DNS resolution failed for '{}': {}", domain_owned, e),
            })?
            .collect();

        if addrs.is_empty() {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("DNS returned no addresses for '{}'", domain_owned),
            });
        }

        for addr in &addrs {
            if is_private_ip(&addr.ip()) {
                return Err(AgentError::ToolExecution {
                    tool_name: "web_fetch".into(),
                    message: format!(
                        "Host '{}' resolves to private/internal address {}",
                        domain_owned,
                        addr.ip()
                    ),
                });
            }
        }
        safe_addrs = addrs;

        return Ok(ValidatedUrl {
            url: parsed,
            domain: Some(domain_owned),
            port,
            resolved_addrs: safe_addrs,
        });
    }

    Ok(ValidatedUrl {
        url: parsed,
        domain: None,
        port,
        resolved_addrs: safe_addrs,
    })
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page by URL and return its text content. \
         Useful for reading documentation, APIs, or any publicly accessible web page."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum characters to return. Default: 10000"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            url: String,
            #[serde(default = "default_max")]
            max_length: usize,
        }
        fn default_max() -> usize {
            10000
        }

        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "web_fetch".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        // SSRF validation — resolve DNS once, validate IPs, then pin them
        // so reqwest cannot re-resolve to a different (malicious) address.
        let validated = validate_url_not_internal(&args.url)?;

        let client = if let Some(ref domain) = validated.domain {
            Self::build_pinned_client(domain, validated.port, &validated.resolved_addrs)
        } else {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("agent-shell/0.1")
                .redirect(ssrf_safe_redirect_policy())
                .build()
                .unwrap_or_default()
        };

        let response = client
            .get(validated.url.as_str())
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();

        // Stream the response body with a hard byte cap to prevent OOM.
        use futures::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buf = Vec::new();
        let mut hit_limit = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Failed to read response body: {}", e),
            })?;
            buf.extend_from_slice(&chunk);
            if buf.len() >= MAX_BODY_BYTES {
                buf.truncate(MAX_BODY_BYTES);
                hit_limit = true;
                break;
            }
        }

        let body = String::from_utf8_lossy(&buf);
        let truncated = if body.len() > args.max_length {
            format!(
                "{}... [truncated, {} total chars{}]",
                &body[..args.max_length],
                body.len(),
                if hit_limit {
                    ", response exceeded 2MB limit"
                } else {
                    ""
                },
            )
        } else if hit_limit {
            format!("{}... [truncated at 2MB limit]", body)
        } else {
            body.into_owned()
        };

        Ok(format!("HTTP {}\n\n{}", status.as_u16(), truncated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // ── is_private_ip unit tests ────────────────────────────────────

    #[test]
    fn test_loopback_v4() {
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn test_loopback_v6() {
        assert!(is_private_ip(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_rfc1918_ranges() {
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    }

    #[test]
    fn test_link_local() {
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
    }

    #[test]
    fn test_cgn_range() {
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
    }

    #[test]
    fn test_public_ip() {
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn test_ipv6_ula() {
        // fc00::/7 — unique local address
        let ip: Ipv6Addr = "fd12:3456:789a::1".parse().unwrap();
        assert!(is_private_ip(&IpAddr::V6(ip)));
    }

    #[test]
    fn test_ipv4_mapped_v6() {
        // ::ffff:127.0.0.1
        let ip: Ipv6Addr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_private_ip(&IpAddr::V6(ip)));
    }

    // ── validate_url_not_internal unit tests ────────────────────────

    #[test]
    fn test_valid_https_url() {
        let result = validate_url_not_internal("https://example.com");
        assert!(result.is_ok());
    }

    #[test]
    fn test_blocked_scheme_ftp() {
        let result = validate_url_not_internal("ftp://example.com/file");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not allowed"), "got: {msg}");
    }

    #[test]
    fn test_blocked_scheme_file() {
        let result = validate_url_not_internal("file:///etc/passwd");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not allowed"), "got: {msg}");
    }

    #[test]
    fn test_blocked_localhost() {
        let result = validate_url_not_internal("http://localhost/secret");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("blocked"), "got: {msg}");
    }

    #[test]
    fn test_blocked_dot_local() {
        let result = validate_url_not_internal("http://foo.local/api");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("blocked"), "got: {msg}");
    }

    #[test]
    fn test_blocked_dot_internal() {
        let result = validate_url_not_internal("http://foo.internal/api");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("blocked"), "got: {msg}");
    }

    #[test]
    fn test_blocked_metadata_endpoint() {
        let result = validate_url_not_internal("http://metadata.google.internal/v1/metadata");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("blocked"), "got: {msg}");
    }

    #[test]
    fn test_blocked_private_ip_v4() {
        let result = validate_url_not_internal("http://192.168.1.1/admin");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn test_blocked_loopback_ip() {
        let result = validate_url_not_internal("http://127.0.0.1:8080/");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    #[test]
    fn test_blocked_ipv6_loopback() {
        let result = validate_url_not_internal("http://[::1]:8080/");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("private"), "got: {msg}");
    }

    // ── redirect policy unit tests ────────────────────────────────────

    #[test]
    fn test_ssrf_redirect_policy_is_custom() {
        // Ensure the custom redirect policy can be constructed without panic.
        let _policy = ssrf_safe_redirect_policy();
    }

    #[test]
    fn test_redirect_target_to_private_ip_blocked() {
        // Validate that internal URLs would be caught during redirect validation.
        let result = validate_url_not_internal("http://127.0.0.1/secret");
        assert!(result.is_err(), "redirect to loopback should be blocked");

        let result = validate_url_not_internal("http://10.0.0.1/internal");
        assert!(result.is_err(), "redirect to RFC1918 should be blocked");

        let result = validate_url_not_internal("http://169.254.169.254/metadata");
        assert!(result.is_err(), "redirect to link-local should be blocked");
    }
}
