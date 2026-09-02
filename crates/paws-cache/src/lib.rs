//! Reports which [`paws_dagger::CacheBackend`] `paws_dagger`'s own
//! `restore_cache_backend`/`save_cache_backend` would actually select right
//! now, and how — the driver behind `paws cache`. Kept as its own crate
//! (not folded into `paws-cli-core`) so it's independently usable (e.g.
//! from `paws-mcp` or a future health-check subsystem) and so `paws-dagger`
//! itself never needs a `serde` dependency just to support one reporting
//! path — this crate owns the presentation, `paws-dagger` owns the
//! mechanism, and this crate never re-derives selection logic: it always
//! wraps `CacheBackend::detect()` directly, so it can't drift from what a
//! real `paws docker`/`paws ci` invocation would do.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    DaggerCloud,
    GithubActions,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheStatus {
    pub backend: Backend,
    /// Only set for `Backend::GithubActions` — "v1" or "v2"
    /// ([`paws_dagger::CacheApiVersion`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    /// Only set for `Backend::GithubActions` — `$ACTIONS_CACHE_URL`/
    /// `$ACTIONS_RESULTS_URL`'s value. Never the runtime token: that's a
    /// bearer credential and has no reason to appear in a status report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl CacheStatus {
    /// Detects the backend from the current process environment — the same
    /// detection `paws_dagger::restore_cache_backend` uses internally.
    pub fn detect() -> Self {
        Self::from(paws_dagger::CacheBackend::detect())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("CacheStatus fields are all JSON-safe")
    }

    pub fn to_text(&self) -> String {
        match self.backend {
            Backend::DaggerCloud => "cache backend: dagger-cloud".to_string(),
            Backend::GithubActions => {
                let version = self.api_version.as_deref().unwrap_or("unknown");
                self.base_url.as_ref().map_or_else(
                    || format!("cache backend: github-actions ({version})"),
                    |url| format!("cache backend: github-actions ({version})\n  {url}"),
                )
            }
            Backend::None => "cache backend: none (full rebuild)".to_string(),
        }
    }
}

impl From<paws_dagger::CacheBackend> for CacheStatus {
    fn from(backend: paws_dagger::CacheBackend) -> Self {
        match backend {
            paws_dagger::CacheBackend::DaggerCloud => Self {
                backend: Backend::DaggerCloud,
                api_version: None,
                base_url: None,
            },
            paws_dagger::CacheBackend::GitHubActionsCache {
                base_url, version, ..
            } => Self {
                backend: Backend::GithubActions,
                api_version: Some(
                    match version {
                        paws_dagger::CacheApiVersion::V1 => "v1",
                        paws_dagger::CacheApiVersion::V2 => "v2",
                    }
                    .to_string(),
                ),
                base_url: Some(base_url),
            },
            paws_dagger::CacheBackend::None => Self {
                backend: Backend::None,
                api_version: None,
                base_url: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dagger_cloud_converts_with_no_extra_fields() {
        let status = CacheStatus::from(paws_dagger::CacheBackend::DaggerCloud);
        assert_eq!(status.backend, Backend::DaggerCloud);
        assert_eq!(status.api_version, None);
        assert_eq!(status.base_url, None);
        assert_eq!(status.to_text(), "cache backend: dagger-cloud");
    }

    #[test]
    fn none_converts_with_no_extra_fields() {
        let status = CacheStatus::from(paws_dagger::CacheBackend::None);
        assert_eq!(status.backend, Backend::None);
        assert_eq!(status.to_text(), "cache backend: none (full rebuild)");
    }

    #[test]
    fn github_actions_v2_carries_version_and_base_url_not_the_token() {
        let status = CacheStatus::from(paws_dagger::CacheBackend::GitHubActionsCache {
            base_url: "https://results.example.invalid/AbC123/".to_string(),
            token: "super-secret-token".to_string(),
            version: paws_dagger::CacheApiVersion::V2,
        });
        assert_eq!(status.backend, Backend::GithubActions);
        assert_eq!(status.api_version.as_deref(), Some("v2"));
        assert_eq!(
            status.base_url.as_deref(),
            Some("https://results.example.invalid/AbC123/")
        );
        let json = status.to_json();
        assert!(!json.contains("super-secret-token"));
        assert!(json.contains("\"v2\""));
    }

    #[test]
    fn json_omits_api_version_and_base_url_for_dagger_cloud_and_none() {
        let json = CacheStatus::from(paws_dagger::CacheBackend::DaggerCloud).to_json();
        assert!(!json.contains("api_version"));
        assert!(!json.contains("base_url"));
    }
}
