//! URL policy: address-bar input resolution, internal pages, navigation scheme rules,
//! HTTPS upgrades. Pure functions; no engine or UI dependencies.

use crate::config::SearchEngine;
use serde::Serialize;
use std::collections::HashSet;
use std::net::IpAddr;
use url::Url;

pub const NEW_TAB_URL: &str = "veyscape://newtab";

/// Pages rendered by the trusted shell. They are never loaded inside a web content view.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InternalPage {
    NewTab,
    Settings,
    History,
    Bookmarks,
    Downloads,
}

pub fn internal_page(url: &str) -> Option<InternalPage> {
    let rest = url.strip_prefix("veyscape://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    Some(match host {
        "newtab" => InternalPage::NewTab,
        "settings" => InternalPage::Settings,
        "history" => InternalPage::History,
        "bookmarks" => InternalPage::Bookmarks,
        "downloads" => InternalPage::Downloads,
        _ => return None,
    })
}

/// Parses an http(s) URL with a host. Anything else returns `None`.
pub fn web_url(s: &str) -> Option<Url> {
    let url = Url::parse(s).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty()))
        .then_some(url)
}

/// Schemes a page may never navigate a tab to, and that are never handed to the OS.
/// `ms-*` covers ms-msdt, ms-officecmd and similar handlers with a history of abuse.
pub fn is_forbidden_scheme(scheme: &str) -> bool {
    let s = scheme.to_ascii_lowercase();
    matches!(
        s.as_str(),
        "file"
            | "javascript"
            | "vbscript"
            | "data"
            | "veyscape"
            | "chrome"
            | "edge"
            | "devtools"
            | "view-source"
            | "search-ms"
            | "search"
            | "shell"
            | "hcp"
            | "its"
            | "mk"
            | "res"
            | "jar"
            | "ldap"
            | "tauri"
            | "ipc"
            | "asset"
    ) || s.starts_with("ms-")
        // microsoft-edge: opens Edge with the user's Microsoft profile (an Edge surface).
        || s.starts_with("microsoft-edge")
}

#[derive(Debug, PartialEq, Eq)]
pub enum NavigationPolicy {
    Allow,
    /// Cancel the navigation; the reason is shown to nobody but logs.
    Block(&'static str),
}

/// Decides whether a top-level navigation inside a web content view may proceed.
/// Unknown but non-forbidden schemes are allowed through so the engine raises its
/// external-protocol event, where the user is asked before anything is launched.
pub fn navigation_policy(url: &str) -> NavigationPolicy {
    if url == "about:blank" || url.starts_with("about:blank#") {
        return NavigationPolicy::Allow;
    }
    let Ok(parsed) = Url::parse(url) else {
        return NavigationPolicy::Block("unparseable URL");
    };
    match parsed.scheme() {
        "http" | "https" | "blob" => NavigationPolicy::Allow,
        "about" => NavigationPolicy::Block("about: pages other than blank"),
        s if is_forbidden_scheme(s) => NavigationPolicy::Block("forbidden scheme"),
        _ => NavigationPolicy::Allow,
    }
}

fn is_local_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".lan")
        || h.ends_with(".internal")
        || h.ends_with(".home.arpa")
        || ip_literal(&h).is_some_and(is_local_ip)
}

/// `10.0.0.1`, `[::1]` (URL hosts keep the brackets) → the address.
fn ip_literal(host: &str) -> Option<IpAddr> {
    let h = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));
    h.unwrap_or(host).parse().ok()
}

/// Only addresses that cannot be reached over the public internet; a public IP literal still gets
/// HTTPS-only. CGNAT (100.64/10) is deliberately not local: it is often a carrier-side address.
fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local_ip(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                v6.is_loopback() || first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00
            }
        },
    }
}

/// HTTPS-only mode: returns the https:// variant of a plain-http URL to try first.
/// Local/LAN hosts are left alone since they rarely have certificates.
pub fn https_upgrade(url: &str) -> Option<String> {
    let mut u = Url::parse(url).ok()?;
    if u.scheme() != "http" || is_local_host(u.host_str()?) {
        return None;
    }
    u.set_scheme("https").ok()?;
    if u.port() == Some(80) {
        u.set_port(None).ok()?;
    }
    Some(u.into())
}

#[derive(Debug, PartialEq, Eq)]
pub enum HttpsOnlyStep {
    /// Let the navigation proceed.
    Continue,
    /// Cancel it and load this https:// URL instead.
    Upgrade(String),
    /// Cancel it and offer plain http to the user: the upgraded navigation came back to http.
    Failed,
}

/// HTTPS-only mode's decision for a top-level navigation to `url`. `upgrading` is true when this
/// navigation already carries an upgrade: an `https -> http` redirect raises the same navigation
/// again, and upgrading that once more would loop forever.
pub fn https_only_step(
    url: &str,
    upgrading: bool,
    http_allowed: &HashSet<String>,
) -> HttpsOnlyStep {
    let Some(upgraded) = https_upgrade(url) else {
        return HttpsOnlyStep::Continue;
    };
    if host_of(url).is_some_and(|h| http_allowed.contains(&h)) {
        HttpsOnlyStep::Continue
    } else if upgrading {
        HttpsOnlyStep::Failed
    } else {
        HttpsOnlyStep::Upgrade(upgraded)
    }
}

/// Turns address-bar text into a URL to load: an internal page, a web URL, or a search.
pub fn resolve_input(input: &str, engine: SearchEngine) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    if internal_page(text).is_some() || text == "about:blank" {
        return Some(text.to_string());
    }
    if !text.contains(char::is_whitespace) {
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            if let Some(u) = web_url(text) {
                return Some(u.into());
            }
        } else if lower.starts_with("mailto:") || lower.starts_with("tel:") {
            return Some(text.to_string());
        } else if let Some(u) = looks_like_host(text) {
            return Some(u);
        }
    }
    let query: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
    Some(engine.template().replace("{}", &query))
}

/// `example.com/path`, `localhost:3000`, `192.168.1.1` → a URL; `hello`, `what is rust?` → `None`.
fn looks_like_host(text: &str) -> Option<String> {
    let host_part = text.split(['/', '?', '#']).next()?;
    // Colons inside a bracketed IPv6 literal are not a port separator; only one after `]` is.
    let port_at = host_part
        .rfind(':')
        .filter(|&i| host_part.rfind(']').is_none_or(|end| i > end));
    let host_only = match port_at {
        Some(i) => {
            let port = &host_part[i + 1..];
            if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            &host_part[..i]
        }
        None => host_part,
    };
    let ip = ip_literal(host_only).is_some();
    let local = is_local_host(host_only);
    // Public Suffix List check, so `node.js` searches while `example.co.uk` navigates.
    let known_suffix = host_only.contains('.')
        && addr::parse_domain_name(host_only).is_ok_and(|n| n.has_known_suffix());
    if !(local || ip || known_suffix) {
        return None;
    }
    let scheme = if local || ip { "http" } else { "https" };
    web_url(&format!("{scheme}://{text}")).map(Into::into)
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    Secure,
    Insecure,
    Internal,
    None,
}

pub fn security_of(url: &str) -> Security {
    if internal_page(url).is_some() {
        Security::Internal
    } else if url.starts_with("https://") {
        Security::Secure
    } else if url.starts_with("http://") {
        Security::Insecure
    } else {
        Security::None
    }
}

/// `https://sub.example.com:8443` style origin used as the key for site permissions.
pub fn origin_of(url: &str) -> Option<String> {
    let u = web_url(url)?;
    Some(u.origin().ascii_serialization())
}

/// Lowercase host used for per-site ad-block exceptions.
pub fn host_of(url: &str) -> Option<String> {
    web_url(url)?.host_str().map(|h| h.to_ascii_lowercase())
}

/// Google's results page preloads a hidden YouTube player (for hover previews of video results).
/// Starting it stalls scrolling and input for ~450 ms in every Chromium browser; measured in Chrome
/// with a clean profile too (docs/performance.md). Veyscape answers that request with an empty
/// response; video results still open on YouTube when clicked.
pub fn is_search_preview_player(page_url: &str, request_url: &str) -> bool {
    // Called for every request: skip parsing unless it can match.
    if !request_url.contains("youtube.com/embed/") {
        return false;
    }
    let (Some(page), Some(request)) = (web_url(page_url), web_url(request_url)) else {
        return false;
    };
    let google_search = page
        .host_str()
        .is_some_and(|h| h.starts_with("www.google.") || h.starts_with("google."))
        && page.path() == "/search";
    google_search
        && matches!(request.host_str(), Some("www.youtube.com" | "youtube.com"))
        && request.path().starts_with("/embed/")
}

/// Inline completion only makes sense for something address-like being typed.
pub fn can_complete(typed: &str) -> bool {
    !typed.is_empty() && !typed.chars().any(char::is_whitespace)
}

/// Address-bar inline completion: what `typed` would complete to for each candidate URL. Scheme and
/// `www.` are skipped unless typed; before any `/` only the site is completed, after it the address.
fn completion_for<'a>(typed: &str, url: &'a str) -> Option<&'a str> {
    let mut s = url;
    if !typed.contains("://") {
        s = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))?;
        if !typed.to_ascii_lowercase().starts_with("www.") {
            s = s.strip_prefix("www.").unwrap_or(s);
        }
    }
    if !typed.contains('/') {
        let host_end = s.find(['/', '?', '#']).unwrap_or(s.len());
        s = &s[..host_end];
    }
    let matches = s.get(..typed.len())?.eq_ignore_ascii_case(typed);
    (matches && s.len() > typed.len()).then_some(s)
}

/// The typed text completed to the site (or address) the user visits most among `candidates`
/// (url, visits), keeping the user's own characters. `None` when nothing extends it.
pub fn inline_completion(typed: &str, candidates: &[(String, i64)]) -> Option<String> {
    let mut scores: Vec<(&str, i64)> = Vec::new();
    for (url, visits) in candidates {
        let Some(text) = completion_for(typed, url) else {
            continue;
        };
        match scores
            .iter_mut()
            .find(|(t, _)| t.eq_ignore_ascii_case(text))
        {
            Some(entry) => entry.1 += visits,
            None => scores.push((text, *visits)),
        }
    }
    // First of the most visited, so ties keep the candidates' order.
    let best = scores
        .iter()
        .fold(None::<(&str, i64)>, |best, &(t, v)| match best {
            Some((_, bv)) if bv >= v => best,
            _ => Some((t, v)),
        })?
        .0;
    Some(format!("{typed}{}", &best[typed.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_preview_player_on_google_search_is_skipped() {
        let serp = "https://www.google.com/search?q=boxing";
        let player = "https://www.youtube.com/embed/?enablejsapi=1&autoplay=0";
        assert!(is_search_preview_player(serp, player));
        assert!(is_search_preview_player(
            "https://www.google.co.uk/search?q=x",
            player
        ));
        // Other Google pages, other sites embedding YouTube, and YouTube itself are untouched.
        assert!(!is_search_preview_player(
            "https://www.google.com/maps",
            player
        ));
        assert!(!is_search_preview_player(
            "https://news.example/search",
            player
        ));
        assert!(!is_search_preview_player(
            "https://www.google.evil.example/search",
            "https://www.youtube.com/watch?v=1"
        ));
        assert!(!is_search_preview_player(
            serp,
            "https://www.youtube.com/watch?v=abc"
        ));
        assert!(!is_search_preview_player(
            serp,
            "https://i.ytimg.com/vi/abc/hq.jpg"
        ));
    }

    fn r(input: &str) -> String {
        resolve_input(input, SearchEngine::DuckDuckGo).unwrap()
    }

    #[test]
    fn resolves_urls_hosts_and_searches() {
        assert_eq!(r("https://example.com"), "https://example.com/");
        assert_eq!(r("example.com"), "https://example.com/");
        assert_eq!(r("example.com/a?b=1"), "https://example.com/a?b=1");
        assert_eq!(r("localhost:3000"), "http://localhost:3000/");
        assert_eq!(r("192.168.1.1"), "http://192.168.1.1/");
        assert_eq!(r("8.8.8.8"), "http://8.8.8.8/");
        assert_eq!(r("[::1]"), "http://[::1]/");
        assert_eq!(r("[fe80::1]"), "http://[fe80::1]/");
        assert_eq!(r("[::1]:8080/x"), "http://[::1]:8080/x");
        assert_eq!(r("http://[::1]"), "http://[::1]/");
        assert_eq!(r("[::1]:"), "https://duckduckgo.com/?q=%5B%3A%3A1%5D%3A");
        assert_eq!(r("veyscape://settings"), "veyscape://settings");
        assert_eq!(r("rust lang"), "https://duckduckgo.com/?q=rust+lang");
        assert_eq!(r("hello"), "https://duckduckgo.com/?q=hello");
        assert_eq!(r("node.js"), "https://duckduckgo.com/?q=node.js");
        assert_eq!(r("bbc.co.uk"), "https://bbc.co.uk/");
        assert_eq!(r("a&b=c"), "https://duckduckgo.com/?q=a%26b%3Dc");
        assert!(resolve_input("   ", SearchEngine::DuckDuckGo).is_none());
    }

    #[test]
    fn dangerous_typed_schemes_become_searches() {
        for input in [
            "javascript:alert(1)",
            "file:///C:/Windows/win.ini",
            "data:text/html,<b>x",
            "ms-msdt:/id",
        ] {
            assert!(
                r(input).starts_with("https://duckduckgo.com/?q="),
                "{input}"
            );
        }
    }

    #[test]
    fn navigation_policy_blocks_forbidden_schemes() {
        assert_eq!(
            navigation_policy("https://a.example/"),
            NavigationPolicy::Allow
        );
        assert_eq!(navigation_policy("about:blank"), NavigationPolicy::Allow);
        assert_eq!(
            navigation_policy("mailto:a@b.example"),
            NavigationPolicy::Allow
        );
        for url in [
            "file:///C:/",
            "veyscape://settings",
            "ms-msdt:x",
            "javascript:1",
            "edge://settings",
            "about:config",
            "microsoft-edge:https://a.example/",
            "Microsoft-Edge-Holographic:x",
        ] {
            assert!(
                matches!(navigation_policy(url), NavigationPolicy::Block(_)),
                "{url}"
            );
        }
    }

    #[test]
    fn https_upgrade_skips_local_hosts() {
        assert_eq!(
            https_upgrade("http://example.com/x").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(
            https_upgrade("http://example.com:80/").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(https_upgrade("https://example.com/"), None);
        assert_eq!(https_upgrade("http://localhost:8080/"), None);
        assert_eq!(https_upgrade("http://10.0.0.1/"), None);
        assert_eq!(https_upgrade("http://printer.local/"), None);
        for local in [
            "http://127.0.0.1/",
            "http://172.16.5.4/",
            "http://192.168.1.1/",
            "http://169.254.1.1/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:192.168.0.1]/",
        ] {
            assert_eq!(https_upgrade(local), None, "{local}");
        }
        for public in [
            "http://8.8.8.8/",
            "http://172.32.0.1/",
            "http://100.64.0.1/",
            "http://[2001:db8::1]/",
            "http://[::ffff:8.8.8.8]/",
        ] {
            assert!(https_upgrade(public).is_some(), "{public}");
        }
    }

    #[test]
    fn https_only_redirect_back_to_http_fails_instead_of_looping() {
        let none = HashSet::new();
        assert_eq!(
            https_only_step("http://a.example/", false, &none),
            HttpsOnlyStep::Upgrade("https://a.example/".into())
        );
        // The upgraded navigation answered `301 -> http://a.example/`.
        assert_eq!(
            https_only_step("http://a.example/", true, &none),
            HttpsOnlyStep::Failed
        );
        assert_eq!(
            https_only_step("https://a.example/", true, &none),
            HttpsOnlyStep::Continue
        );
        assert_eq!(
            https_only_step("http://localhost/", true, &none),
            HttpsOnlyStep::Continue
        );
        let allowed = HashSet::from(["a.example".to_string()]);
        for upgrading in [false, true] {
            assert_eq!(
                https_only_step("http://A.example/x", upgrading, &allowed),
                HttpsOnlyStep::Continue
            );
        }
    }

    #[test]
    fn origins_hosts_and_security() {
        assert_eq!(
            origin_of("https://A.example:8443/x").as_deref(),
            Some("https://a.example:8443")
        );
        assert_eq!(
            host_of("https://WWW.Example.com/").as_deref(),
            Some("www.example.com")
        );
        assert_eq!(origin_of("veyscape://settings"), None);
        assert_eq!(security_of("http://a.example/"), Security::Insecure);
        assert_eq!(security_of(NEW_TAB_URL), Security::Internal);
        assert_eq!(
            internal_page("veyscape://settings/privacy"),
            Some(InternalPage::Settings)
        );
        assert_eq!(internal_page("veyscape://unknown"), None);
    }

    #[test]
    fn inline_completion_picks_the_most_visited_site() {
        let c = |list: &[(&str, i64)]| -> Vec<(String, i64)> {
            list.iter().map(|(u, v)| (u.to_string(), *v)).collect()
        };
        let visits = c(&[
            ("https://www.github.com/login", 3),
            ("https://github.com/explore", 2),
            ("https://gitlab.com/", 4),
            ("http://example.com/", 9),
        ]);
        // github.com: 3 + 2 visits across two pages beats gitlab.com's 4.
        assert_eq!(
            inline_completion("git", &visits).as_deref(),
            Some("github.com")
        );
        assert_eq!(
            inline_completion("GitL", &visits).as_deref(),
            Some("GitLab.com")
        );
        assert_eq!(
            inline_completion("github.com/l", &visits).as_deref(),
            Some("github.com/login")
        );
        assert_eq!(
            inline_completion("www.gith", &visits).as_deref(),
            Some("www.github.com")
        );
        assert_eq!(
            inline_completion("https://gitl", &visits).as_deref(),
            Some("https://gitlab.com/")
        );
        assert_eq!(inline_completion("github.com", &visits), None);
        assert_eq!(inline_completion("xyz", &visits), None);
        assert!(!can_complete("git hub") && !can_complete("") && can_complete("git"));
    }
}
