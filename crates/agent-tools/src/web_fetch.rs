use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use url::Url;

/// Fetch a web page and return its text content.
pub struct WebFetchTool {
    client: reqwest::Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("agent-shell/0.1")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self { client }
    }
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

/// Validate that a URL is safe to fetch (not an internal/SSRF target).
pub(crate) fn validate_url_not_internal(raw_url: &str) -> Result<Url, AgentError> {
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

    // Resolve DNS and block private IPs.
    // Check if the host is a raw IP address first.
    if let Some(url::Host::Ipv4(ip)) = parsed.host() {
        if is_private_ip(&IpAddr::V4(ip)) {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("IP address {} is a private/internal address", ip),
            });
        }
    }
    if let Some(url::Host::Ipv6(ip)) = parsed.host() {
        if is_private_ip(&IpAddr::V6(ip)) {
            return Err(AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("IP address {} is a private/internal address", ip),
            });
        }
    }

    // For domain names, perform DNS resolution and check all resolved IPs.
    if let Some(url::Host::Domain(domain)) = parsed.host() {
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addr_str = format!("{}:{}", domain, port);
        if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&addr_str) {
            for addr in addrs {
                if is_private_ip(&addr.ip()) {
                    return Err(AgentError::ToolExecution {
                        tool_name: "web_fetch".into(),
                        message: format!(
                            "Host '{}' resolves to private/internal address {}",
                            domain,
                            addr.ip()
                        ),
                    });
                }
            }
        }
    }

    Ok(parsed)
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

        // SSRF validation — block internal URLs before making the request.
        validate_url_not_internal(&args.url)?;

        let response =
            self.client
                .get(&args.url)
                .send()
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "web_fetch".into(),
                    message: format!("Request failed: {}", e),
                })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Failed to read response body: {}", e),
            })?;

        let truncated = if body.len() > args.max_length {
            format!(
                "{}... [truncated, {} total chars]",
                &body[..args.max_length],
                body.len()
            )
        } else {
            body
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
}
