//! Manual update check: one request to GitHub Releases, only when the user presses the button.

use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Newest release first, pre-releases included (every release so far is a preview).
const RELEASES_API: &str = "https://api.github.com/repos/LunoviaVR/Veyscape/releases?per_page=1";
const RELEASES_PAGE: &str = "https://github.com/LunoviaVR/Veyscape/releases";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub latest: String,
    pub available: bool,
    /// Release page to open in a tab; always on the project's GitHub releases.
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// Blocking; run off the UI thread.
pub fn check(current: &str) -> AppResult<UpdateInfo> {
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let body = agent
        .get(RELEASES_API)
        // GitHub rejects API requests without a User-Agent.
        .header("User-Agent", "Veyscape")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(call_error)?
        .into_body()
        .read_to_string()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let releases: Vec<Release> = serde_json::from_str(&body)
        .map_err(|_| AppError::Network("unexpected response from GitHub".into()))?;
    let release = releases
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Network("no releases published".into()))?;
    Ok(info(current, release))
}

/// GitHub allows about 60 anonymous API requests per hour per IP; say so instead of "http status: 403".
fn call_error(e: ureq::Error) -> AppError {
    match e {
        ureq::Error::StatusCode(403 | 429) => AppError::Network(
            "GitHub is limiting update checks from this network. Try again in an hour.".into(),
        ),
        e => AppError::Network(e.to_string()),
    }
}

fn info(current: &str, release: Release) -> UpdateInfo {
    let latest = release.tag_name.trim_start_matches('v').to_owned();
    let available = matches!((parse(&latest), parse(current)), (Some(l), Some(c)) if l > c);
    // The response decides which page opens in a tab, so pin it to our releases.
    let url = if release.html_url.starts_with(&format!("{RELEASES_PAGE}/")) {
        release.html_url
    } else {
        RELEASES_PAGE.to_owned()
    };
    UpdateInfo {
        latest,
        available,
        url,
    }
}

/// `1.2.3` (any `-pre`/`+build` suffix ignored) → comparable tuple.
fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, url: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: url.into(),
        }
    }

    #[test]
    fn compares_numerically() {
        let r = |tag: &str| release(tag, "https://github.com/LunoviaVR/Veyscape/releases/tag/x");
        assert!(info("0.1.1", r("v0.1.10")).available);
        assert!(info("0.1.1", r("v0.2.0-beta")).available);
        assert!(!info("0.1.1", r("v0.1.1")).available);
        assert!(!info("0.2.0", r("v0.1.9")).available);
        assert!(!info("0.1.1", r("nightly")).available);
        assert_eq!(info("0.1.1", r("v0.1.2")).latest, "0.1.2");
    }

    #[test]
    fn rate_limit_is_explained() {
        let msg = call_error(ureq::Error::StatusCode(403)).to_string();
        assert!(msg.contains("Try again in an hour"), "{msg}");
        let other = call_error(ureq::Error::StatusCode(500)).to_string();
        assert!(other.contains("http status: 500"), "{other}");
    }

    #[test]
    fn only_opens_project_release_pages() {
        let ok = "https://github.com/LunoviaVR/Veyscape/releases/tag/v0.1.2";
        assert_eq!(info("0.1.1", release("v0.1.2", ok)).url, ok);
        let evil = info("0.1.1", release("v0.1.2", "https://evil.example/"));
        assert_eq!(evil.url, RELEASES_PAGE);
        let lookalike = info(
            "0.1.1",
            release(
                "v0.1.2",
                "https://github.com/LunoviaVR/Veyscape/releases.evil.example",
            ),
        );
        assert_eq!(lookalike.url, RELEASES_PAGE);
    }
}
