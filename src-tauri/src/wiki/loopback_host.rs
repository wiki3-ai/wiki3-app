//! Resolve the loopback hostname to use for local site URLs.
//!
//! We prefer a human-friendly name from `/etc/hosts` over the bare
//! `127.0.0.1` literal, both because it's nicer in the address bar
//! and because some browsers / cookies behave better with a name.
//!
//! Order of preference:
//! 1. The first hostname mapped to `::1` in `/etc/hosts`
//! 2. The first hostname mapped to `127.0.0.1` in `/etc/hosts`
//! 3. The literal `"localhost"`
//!
//! Computed once and cached for the life of the process.

use std::sync::OnceLock;

static CACHED: OnceLock<String> = OnceLock::new();

const HOSTS_PATH: &str = "/etc/hosts";

/// Return the loopback hostname to use in URLs (e.g. `localhost`).
pub fn loopback_hostname() -> &'static str {
    CACHED
        .get_or_init(|| {
            let contents = std::fs::read_to_string(HOSTS_PATH).unwrap_or_default();
            resolve_from_hosts(&contents).unwrap_or_else(|| "localhost".to_string())
        })
        .as_str()
}

/// Parse `/etc/hosts` content and pick a hostname per the preference
/// described at the module level. Pure for testability.
fn resolve_from_hosts(contents: &str) -> Option<String> {
    let mut v6: Option<String> = None;
    let mut v4: Option<String> = None;

    for line in contents.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut tokens = trimmed.split_whitespace();
        let Some(addr) = tokens.next() else { continue };
        let Some(name) = tokens.next() else { continue };
        // Skip names that look syntactically wrong (defensive).
        if name.is_empty() {
            continue;
        }
        if v6.is_none() && addr == "::1" {
            v6 = Some(name.to_string());
        } else if v4.is_none() && addr == "127.0.0.1" {
            v4 = Some(name.to_string());
        }
        if v6.is_some() && v4.is_some() {
            break;
        }
    }
    v6.or(v4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_v6_then_v4() {
        let hosts = "\
127.0.0.1\tfoo
::1\tbar
";
        assert_eq!(resolve_from_hosts(hosts).as_deref(), Some("bar"));
    }

    #[test]
    fn falls_back_to_v4_when_no_v6() {
        let hosts = "127.0.0.1\tlocalhost myhost\n";
        assert_eq!(resolve_from_hosts(hosts).as_deref(), Some("localhost"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let hosts = "\
# a comment
   
127.0.0.1   gizmo  # inline comment
";
        assert_eq!(resolve_from_hosts(hosts).as_deref(), Some("gizmo"));
    }

    #[test]
    fn none_when_nothing_matches() {
        let hosts = "10.0.0.1 foo\n";
        assert!(resolve_from_hosts(hosts).is_none());
    }

    #[test]
    fn picks_first_v6_name_only() {
        let hosts = "\
::1 first second
::1 third
";
        assert_eq!(resolve_from_hosts(hosts).as_deref(), Some("first"));
    }
}
