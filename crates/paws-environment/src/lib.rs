//! Normalizes CI-provider context (repo, commit sha, ref, token) behind one
//! type, so subcommands that need it (e.g. `paws semver --push`) don't
//! hand-read `GITHUB_*`/`GITLAB_*` env vars directly at each call site.
//!
//! GitHub Actions is the only implemented provider today. [`Provider`] and
//! [`CiContext::detect`] are shaped so a GitLab CI (or other) provider can be
//! added later as another `detect`/`push_tag` branch, without changing any
//! call site that already holds a [`CiContext`].

use anyhow::{Context, Result, bail};

/// Credentials for a GitHub App, used to mint a short-lived installation
/// access token natively (no `actions/create-github-app-token` needed) —
/// see [`mint_github_app_installation_token`].
pub struct GitHubAppCredentials {
    /// The App's Client ID (the `Iv23...`-style string) — GitHub's docs
    /// recommend this over the legacy numeric App ID for the JWT `iss`
    /// claim.
    pub client_id: String,
    /// The App's private key, PEM-encoded.
    pub private_key_pem: String,
}

/// Hand-written, not derived: `private_key_pem` is the App's signing key. A
/// derived `Debug` would print it in full the first time anyone `{:?}`s this
/// struct into a log or an `anyhow` context, which is exactly the kind of
/// leak that only shows up after the log has already shipped.
impl std::fmt::Debug for GitHubAppCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAppCredentials")
            .field("client_id", &self.client_id)
            .field("private_key_pem", &"<redacted>")
            .finish()
    }
}

#[derive(serde::Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Mints a short-lived (~1 hour) installation access token for `owner/repo`,
/// authenticating as the GitHub App identified by `creds` — the same
/// mechanism `actions/create-github-app-token` uses, done natively so
/// `paws` doesn't need that Action as a separate CI step. A commit made
/// with this token registers as authored by the App, which matters when a
/// branch ruleset's required-status-check rule only bypasses for specific
/// actors (like this App) rather than the default `GITHUB_TOKEN`.
/// Signs the short-lived App-level JWT `mint_github_app_installation_token`
/// authenticates its two GitHub API calls with — split out so it's testable
/// without a network call. This is also the exact code path that hit a real
/// production panic once (`jsonwebtoken`'s default crypto backend needs a
/// process-wide rustls `CryptoProvider` installed before first use, which
/// nothing in this binary did) — see `Cargo.toml`'s `rust_crypto` feature
/// pin, and `jwt_signing_does_not_panic_with_the_configured_crypto_backend`
/// below, which exists specifically to catch a regression of that failure.
fn sign_app_jwt(creds: &GitHubAppCredentials) -> Result<String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    // `jsonwebtoken`'s claim fields are `i64`. `as_secs()` is a `u64`, so a
    // plain `as` would wrap silently — `try_into` turns the (impossible until
    // year 292277026596) overflow into an error instead of a negative `iat`
    // that GitHub would reject with an opaque 401.
    let now: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs()
        .try_into()
        .context("system clock is too far in the future to express as a JWT timestamp")?;
    let claims = AppJwtClaims {
        // 60s in the past to tolerate clock drift between this machine and
        // GitHub's, per GitHub's own JWT-generation guidance.
        iat: now - 60,
        // GitHub caps this at 10 minutes past `iat`; 9 minutes leaves
        // margin without cutting it close.
        exp: now + 9 * 60,
        iss: creds.client_id.clone(),
    };
    let key = EncodingKey::from_rsa_pem(creds.private_key_pem.as_bytes())
        .context("GitHub App private key is not valid PEM")?;
    encode(&Header::new(Algorithm::RS256), &claims, &key)
        .context("failed to sign the GitHub App JWT")
}

pub async fn mint_github_app_installation_token(
    creds: &GitHubAppCredentials,
    owner: &str,
    repo: &str,
) -> Result<String> {
    let jwt = sign_app_jwt(creds)?;

    let client = reqwest::Client::new();
    let auth_headers = |builder: reqwest::RequestBuilder| {
        builder
            .bearer_auth(&jwt)
            .header("User-Agent", "paws-environment")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    };

    let installation: serde_json::Value = auth_headers(client.get(format!(
        "https://api.github.com/repos/{owner}/{repo}/installation"
    )))
    .send()
    .await
    .context("failed to reach GitHub's repository-installation API")?
    .error_for_status()
    .context(
        "GitHub rejected the repository-installation lookup — is the GitHub App actually \
         installed on this repository?",
    )?
    .json()
    .await
    .context("failed to parse GitHub's repository-installation response")?;
    let installation_id = installation
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .context("GitHub's repository-installation response had no \"id\" field")?;

    let access_token: serde_json::Value = auth_headers(client.post(format!(
        "https://api.github.com/app/installations/{installation_id}/access_tokens"
    )))
    .send()
    .await
    .context("failed to reach GitHub's installation access-tokens API")?
    .error_for_status()
    .context("GitHub rejected the installation access-token request")?
    .json()
    .await
    .context("failed to parse GitHub's installation access-token response")?;
    access_token
        .get("token")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .context("GitHub's installation access-token response had no \"token\" field")
}

/// Resolves a GitHub token for `owner/repo`: mints a fresh GitHub App
/// installation token when `$GH_APP_CLIENT_ID` and a private key
/// (`$GH_APP_PRIVATE_KEY` or `$GH_APP_PRIVATE_KEY_FILE`) are both present,
/// otherwise falls back to the plain `$GITHUB_TOKEN`/`$GH_TOKEN` env vars
/// every call site used to read directly. Incomplete App credentials (a
/// client ID with no private key) fall back too, rather than half-attempting
/// App auth — a partially-configured App shouldn't shadow a working
/// `GITHUB_TOKEN`.
pub async fn resolve_github_token(owner: &str, repo: &str) -> Result<String> {
    if let Ok(client_id) = std::env::var("GH_APP_CLIENT_ID") {
        let private_key_pem = if let Ok(pem) = std::env::var("GH_APP_PRIVATE_KEY") {
            Some(pem)
        } else if let Ok(path) = std::env::var("GH_APP_PRIVATE_KEY_FILE") {
            Some(
                std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read $GH_APP_PRIVATE_KEY_FILE ({path})"))?,
            )
        } else {
            None
        };
        if let Some(private_key_pem) = private_key_pem {
            let creds = GitHubAppCredentials {
                client_id,
                private_key_pem,
            };
            return mint_github_app_installation_token(&creds, owner, repo).await;
        }
    }

    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .context(
            "GITHUB_TOKEN (or GH_TOKEN) must be set — or $GH_APP_CLIENT_ID plus \
             $GH_APP_PRIVATE_KEY/$GH_APP_PRIVATE_KEY_FILE for GitHub App auth",
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
}

/// Normalized CI context: which provider is running this, and the
/// repository/commit/token it gave us.
#[derive(Debug, Clone)]
pub struct CiContext {
    pub provider: Provider,
    pub owner: String,
    pub repo: String,
    /// Commit SHA being built. Empty outside CI (falls back to `""`, matching
    /// every other subcommand's existing `GITHUB_SHA` handling).
    pub sha: String,
    /// Verbatim ref, e.g. `refs/heads/main` or `refs/tags/v1.2.3`.
    pub git_ref: Option<String>,
    pub token: String,
}

impl CiContext {
    /// Detects the running CI provider from its environment variables.
    /// GitHub Actions only for now (`GITHUB_REPOSITORY` present); other
    /// providers add another `if let Ok(..) = std::env::var(..)` branch here.
    ///
    /// The token comes from [`resolve_github_token`] — a GitHub App
    /// installation token when `$GH_APP_CLIENT_ID`+a private key are
    /// configured, otherwise the plain `$GITHUB_TOKEN`/`$GH_TOKEN` env vars.
    pub async fn detect() -> Result<Self> {
        if let Ok(repository) = std::env::var("GITHUB_REPOSITORY") {
            let (owner, repo) = repository.split_once('/').with_context(|| {
                format!("GITHUB_REPOSITORY {repository:?} is not \"owner/repo\"")
            })?;
            let token = resolve_github_token(owner, repo).await?;
            return Ok(Self {
                provider: Provider::GitHub,
                owner: owner.to_string(),
                repo: repo.to_string(),
                sha: std::env::var("GITHUB_SHA").unwrap_or_default(),
                git_ref: std::env::var("GITHUB_REF").ok(),
                token,
            });
        }

        bail!("no supported CI provider detected (checked: GitHub Actions via GITHUB_REPOSITORY)")
    }
}

/// Identity a pushed tag is attributed to (GitHub's annotated-tag `tagger`
/// field). Defaults to a `paws`-branded bot identity distinct from whatever
/// token/app actually authenticates the API call.
#[derive(Debug, Clone)]
pub struct TagAuthor<'a> {
    pub name: &'a str,
    pub email: &'a str,
}

impl Default for TagAuthor<'static> {
    fn default() -> Self {
        Self {
            name: "paws-bot",
            email: "paws-bot@users.noreply.github.com",
        }
    }
}

/// Creates an annotated tag pointing at `ctx.sha` and pushes it, then
/// creates the matching GitHub Release (`gh-reusable`'s original
/// `tagger.yaml` did both — `git tag && git push` followed by `gh release
/// create --generate-notes` — so a bare tag push alone would be an
/// incomplete replacement). No local git identity/worktree required
/// (matches `paws-release::GitHubReleaseClient`'s Contents-API-over-git
/// precedent). If the release creation fails after the tag already landed,
/// that error is still returned — the tag itself is not rolled back, since
/// `git push`-ing a tag isn't an atomic pair with creating a Release either
/// in the system this replaces.
pub async fn push_tag(ctx: &CiContext, tag: &str, author: &TagAuthor<'_>) -> Result<()> {
    match ctx.provider {
        Provider::GitHub => {
            push_tag_github(ctx, tag, author).await?;
            create_release_github(ctx, tag).await
        }
    }
}

async fn push_tag_github(ctx: &CiContext, tag: &str, author: &TagAuthor<'_>) -> Result<()> {
    let client = reqwest::Client::new();
    let base = format!(
        "https://api.github.com/repos/{}/{}/git",
        ctx.owner, ctx.repo
    );

    let tag_object: serde_json::Value = client
        .post(format!("{base}/tags"))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "tag": tag,
            "message": tag,
            "object": ctx.sha,
            "type": "commit",
            "tagger": { "name": author.name, "email": author.email },
        }))
        .send()
        .await
        .context("failed to reach GitHub's git/tags API")?
        .error_for_status()
        .context("GitHub rejected the tag object")?
        .json()
        .await
        .context("failed to parse GitHub's tag-object response")?;

    let tag_sha = tag_object
        .get("sha")
        .and_then(|v| v.as_str())
        .context("GitHub's tag-object response had no \"sha\" field")?;

    client
        .post(format!("{base}/refs"))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "ref": format!("refs/tags/{tag}"),
            "sha": tag_sha,
        }))
        .send()
        .await
        .context("failed to reach GitHub's git/refs API")?
        .error_for_status()
        .context("GitHub rejected the tag ref")?;

    Ok(())
}

/// Creates a GitHub Release for an already-pushed tag, with auto-generated
/// notes — matches `gh-reusable`'s `tagger.yaml` (`gh release create "$TAG"
/// --title "Release $TAG" --generate-notes`) exactly, including the
/// `Release {tag}` title convention.
async fn create_release_github(ctx: &CiContext, tag: &str) -> Result<()> {
    reqwest::Client::new()
        .post(format!(
            "https://api.github.com/repos/{}/{}/releases",
            ctx.owner, ctx.repo
        ))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "tag_name": tag,
            "name": format!("Release {tag}"),
            "generate_release_notes": true,
        }))
        .send()
        .await
        .context("failed to reach GitHub's releases API")?
        .error_for_status()
        .context("GitHub rejected the release")?;

    Ok(())
}

#[cfg(test)]
// `std::env::set_var`/`remove_var` are unsafe in edition 2024, and these
// tests exist precisely to exercise env-var-driven detection. Every call is
// serialized behind `ENV_LOCK` below, which is what makes it sound.
#[allow(unsafe_code)]
mod tests {
    /// `missing_debug_implementations` forced a choice here, and the choice was
    /// a redacting impl rather than a derive. This pins it: a derived `Debug`
    /// would print the App's signing key in full.
    #[test]
    fn debug_never_prints_the_private_key() {
        let creds = GitHubAppCredentials {
            client_id: "Iv23liExampleClientId".to_string(),
            private_key_pem: "-----BEGIN RSA PRIVATE KEY-----\nSUPERSECRETKEYMATERIAL\n"
                .to_string(),
        };
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("SUPERSECRETKEYMATERIAL"),
            "private key leaked into Debug output: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
        // The non-secret half stays useful for debugging.
        assert!(rendered.contains("Iv23liExampleClientId"));
    }

    use super::*;
    use base64::Engine as _;

    #[test]
    fn default_tag_author_is_paws_bot() {
        let author = TagAuthor::default();
        assert_eq!(author.name, "paws-bot");
        assert_eq!(author.email, "paws-bot@users.noreply.github.com");
    }

    // A throwaway 2048-bit RSA key generated locally for this test only
    // (`openssl genrsa -traditional 2048`) — never used for anything real,
    // safe to commit. PKCS#1 ("BEGIN RSA PRIVATE KEY"), matching the format
    // GitHub Apps' own downloaded private keys use.
    const TEST_RSA_PRIVATE_KEY_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAqV3omqp0j9pjIgY9NBnQOh7dGFJ/v9P+D7O4cGdRXtLi7tHW
g97qQcrjuCt29nC4oeKjwxVYP71/Q+ZDwGGe/1HSMY1koP18jtHCnZaGnrfgTk8T
T6U1Jn6RyYygL5z7/EWbLthwhEkDH6pRr38LWBjavahDMldOWnsZRqxPKgsHCIi5
R+nxU4dK130yl+QBwumuD1HggB5jeL3wj1MNGixyoYiiX5N5PZRuzTXEIcq4MtGO
AzEmIFN9dap/TmDiuyCxbPbekHHE2CpLO1sbxL/d87Zwx8k4YExlHGGxmsWEdwlz
oVhnkjQYH4DRZAYTUW/+eR8S8hxeeDBp6xG5QwIDAQABAoIBAC/16l0GAP0NhD4J
00ISPz9+JvDwx8FMKGlM5OFbuJSoFmA3ps3wDZk0+ZhZIpZ15Crfka04OaXPJR9W
sP/lBQ/bHTEwD3txXNjauIhErHl8q3Wxec/3gh4VAHa5LlFdXJQbJ+8zlmU3gb1x
TzFpwg4f9612XRT/2S3RJx62w7ItN8s1kxVmr2YcGv76qVIvvr6kHOztgO4B0Swi
8ZDMmowT8QRP0F++BGDNXKj8PfHSB8SQd0IRF365n2lvbTseAnyoNfdisDKtNszl
4yB/qbwEyjzTMelY8OK8OFjQHhM169NdEHVPJM6l01vMQlJ/pTZx43ILxdRbtnk3
VEiGt5kCgYEA5Nghh76yPS429qWFgPnsVNvOz+hKCGliZTSGzcDfoD+PA+C5fvq1
3Noo8G+kaQmZNINH5Kgk0tNcchhiDivw8K++RlhckxEYMy44ofD5qTaItpQBA0VG
2m0idJFW82WCS2AAx8iJ0gokBa9clAxT51xI75N0ik1IcwpWuNDUB3sCgYEAvXb2
XBIg3sCE9ZKUgJn1l+qaj1h00EDoIdsvQbWD93LpAoUt1wO/vOPY/+GrYvFLR+Eq
hng5mN/S02aDdxC/8ZD3BY2/kNQ9n0p+B+nfT0KxF1YkshEE9CHJbV+avg1CaJ0w
6VYcu9A2K+vRo1/1i/GF9sPaf3uRl6vtU/A3htkCgYBJwEHmHpYQ05ERIj0JWQJK
QuC+7mzVkykL1sbPDqbDXVh49naxrpjnyUNCYaiJ1XcTjm+gCHR9oXJ8rtEDIjQv
TWQ0BYwoNW0oKXBE+IVtfE7JEJ/W7v+rq1pcWO692GwKYLE/saiBEZWUY3Shnet4
d6xl0Y7Qd6GuuZlDTMHYewKBgEiga3uLr3Hz1oPUNny9h7k+QxUj0VNrLhCcVpcX
n4ihUdSXfKTpWPxtUudzeCErYbIiDA0T1PBXDBfhOg/QKePNsAM+/OnlkeGXyov6
CJH3fK73ZIWlpIJ42R/GAClOJ+C2MOOhEM6l174qXWgFBrkoUjPvi7hGg97iFs2Q
TZixAoGAS26IcwYSQH7oYFuxyFLVVG5K4mKVW7s6/NSspl8rv/pJKpk/6HmynPpt
nyLOmeNH7f0X0tWR6B87/0i02mQpvK4v1N7MsvUIpQDM8g6zqqq8bRe9uCdTdw17
/wsEW98hxzIdPtvh1CutI+LMVJPQSvz8iDWem64ekQRmZ/3yMOQ=
-----END RSA PRIVATE KEY-----";

    /// Regression test for a real production failure: `jsonwebtoken`'s
    /// default crypto backend needs a process-wide rustls `CryptoProvider`
    /// installed before first use (nothing in this binary did that, and
    /// `reqwest`'s own rustls-tls linkage doesn't auto-install one), so
    /// `sign_app_jwt` panicked at runtime on a real CI run despite building
    /// and unit-testing clean beforehand — the failure mode was entirely
    /// runtime, not caught by `cargo build`/`cargo check`. Fixed by pinning
    /// `jsonwebtoken`'s `rust_crypto` feature (see `Cargo.toml`), which
    /// avoids the global-provider dependency entirely. This test signs a
    /// real JWT with a real (throwaway) RSA key, so it would have caught
    /// that regression directly.
    #[test]
    fn jwt_signing_does_not_panic_with_the_configured_crypto_backend() {
        let creds = GitHubAppCredentials {
            client_id: "Iv23liTestClientId".to_string(),
            private_key_pem: TEST_RSA_PRIVATE_KEY_PEM.to_string(),
        };

        let jwt = sign_app_jwt(&creds).expect("signing should succeed, not panic or error");

        // Decode the claims (no signature verification needed here — this
        // test is about the signing path not panicking/erroring, and about
        // the claims shape being right, not about verifying our own
        // signature) by base64-decoding the JWT's middle segment.
        let payload_b64 = jwt.split('.').nth(1).expect("JWT has a payload segment");
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .expect("payload segment is valid base64url");
        let claims: serde_json::Value =
            serde_json::from_slice(&payload_bytes).expect("payload is valid JSON");

        assert_eq!(claims["iss"], "Iv23liTestClientId");
        let iat = claims["iat"].as_i64().expect("iat is a number");
        let exp = claims["exp"].as_i64().expect("exp is a number");
        assert!(exp > iat, "exp ({exp}) should be after iat ({iat})");
        assert!(
            exp - iat <= 600,
            "exp - iat ({}) should stay within GitHub's 10-minute cap",
            exp - iat
        );
    }

    // Serialized: every test below mutates process-wide env vars
    // (GITHUB_REPOSITORY/GH_APP_*/GITHUB_TOKEN/GH_TOKEN), and `cargo test`
    // runs test fns in this module concurrently on separate threads by
    // default — without this, one test's env mutation can leak into
    // another's assertions.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// SAFETY: guarded by `ENV_LOCK` above — no concurrent env access from
    /// other tests in this module while a guard is held.
    unsafe fn clear_env(vars: &[&str]) {
        for var in vars {
            unsafe {
                std::env::remove_var(var);
            }
        }
    }

    const ENV_VARS_UNDER_TEST: &[&str] = &[
        "GITHUB_REPOSITORY",
        "GH_APP_CLIENT_ID",
        "GH_APP_PRIVATE_KEY",
        "GH_APP_PRIVATE_KEY_FILE",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ];

    #[tokio::test]
    async fn detect_fails_loudly_with_no_ci_env_vars_present() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
        let err = CiContext::detect().await.unwrap_err();
        assert!(err.to_string().contains("no supported CI provider"));
    }

    #[tokio::test]
    async fn resolve_github_token_falls_back_to_plain_env_token_with_no_app_creds() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
            std::env::set_var("GITHUB_TOKEN", "plain-token-value");
        }
        let token = resolve_github_token("owner", "repo").await.unwrap();
        assert_eq!(token, "plain-token-value");
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
    }

    #[tokio::test]
    async fn resolve_github_token_ignores_an_incomplete_app_client_id_and_falls_back() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
            // A client ID with no private key anywhere shouldn't attempt
            // (and fail) App auth — it should fall back to GITHUB_TOKEN.
            std::env::set_var("GH_APP_CLIENT_ID", "Iv23liSomeClientId");
            std::env::set_var("GH_TOKEN", "fallback-token-value");
        }
        let token = resolve_github_token("owner", "repo").await.unwrap();
        assert_eq!(token, "fallback-token-value");
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
    }

    #[tokio::test]
    async fn resolve_github_token_errors_clearly_with_nothing_configured() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
        let err = resolve_github_token("owner", "repo").await.unwrap_err();
        assert!(err.to_string().contains("GITHUB_TOKEN"));
        assert!(err.to_string().contains("GH_APP_CLIENT_ID"));
    }
}

// --- GitHub Actions step outputs ------------------------------------------

/// Format one entry for `$GITHUB_OUTPUT`.
///
/// Single-line values use `key=value`. Anything containing a newline needs the
/// heredoc form, and the delimiter must not appear in the value — otherwise a
/// value could close its own block early and forge further outputs. The
/// delimiter is extended until it is absent from the value rather than assumed
/// unique.
pub fn format_output(key: &str, value: &str) -> String {
    if !value.contains('\n') && !value.contains('\r') {
        return format!("{key}={value}\n");
    }

    let mut delimiter = String::from("paws_eof");
    while value.contains(&delimiter) {
        delimiter.push('_');
    }

    format!("{key}<<{delimiter}\n{value}\n{delimiter}\n")
}

/// Append step outputs to `$GITHUB_OUTPUT`, if it is set.
///
/// A no-op when the variable is absent, so callers never branch on whether
/// they are running under GitHub Actions. Returns whether anything was written.
///
/// Without this, every consumer has to scrape stdout —
/// `version="$(paws semver … | tail -n1)"` — which breaks the moment a
/// subcommand prints one extra line.
pub fn write_outputs(pairs: &[(&str, &str)]) -> std::io::Result<bool> {
    use std::io::Write as _;

    let Ok(path) = std::env::var("GITHUB_OUTPUT") else {
        return Ok(false);
    };
    if path.is_empty() || pairs.is_empty() {
        return Ok(false);
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for (key, value) in pairs {
        file.write_all(format_output(key, value).as_bytes())?;
    }

    Ok(true)
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn a_single_line_value_uses_the_simple_form() {
        assert_eq!(format_output("version", "v1.2.3"), "version=v1.2.3\n");
    }

    #[test]
    fn an_empty_value_is_still_written() {
        // Consumers distinguish "set but empty" from "absent"; dropping it
        // would make those look the same.
        assert_eq!(format_output("tags", ""), "tags=\n");
    }

    #[test]
    fn a_multiline_value_uses_a_heredoc() {
        let formatted = format_output("tags", "ghcr.io/o/a:v1\nghcr.io/o/a:latest");

        assert_eq!(
            formatted,
            "tags<<paws_eof\nghcr.io/o/a:v1\nghcr.io/o/a:latest\npaws_eof\n"
        );
    }

    /// A value containing the delimiter could otherwise close its own block
    /// early and forge whatever outputs followed.
    #[test]
    fn a_value_containing_the_delimiter_gets_a_longer_one() {
        let formatted = format_output("body", "line\npaws_eof\nmore");

        assert!(
            formatted.starts_with("body<<paws_eof_\n"),
            "got {formatted}"
        );
        assert!(formatted.ends_with("\npaws_eof_\n"));
        // The literal in the value must not terminate the block.
        assert!(formatted.contains("\npaws_eof\n"));
    }

    #[test]
    fn the_delimiter_grows_until_it_is_unique() {
        let value = "paws_eof\npaws_eof_\npaws_eof__";
        let formatted = format_output("body", value);

        assert!(
            formatted.starts_with("body<<paws_eof___\n"),
            "got {formatted}"
        );
    }

    #[test]
    fn carriage_returns_also_force_the_heredoc_form() {
        // A bare \r would otherwise be written into a key=value line and
        // truncate it on parse.
        assert!(format_output("v", "a\rb").starts_with("v<<"));
    }

    #[test]
    fn writing_is_a_no_op_without_the_environment_variable() {
        // Safe regardless of how the suite is run: absent or empty both skip.
        if std::env::var("GITHUB_OUTPUT").is_err() {
            assert!(!write_outputs(&[("k", "v")]).unwrap());
        }
        assert!(!write_outputs(&[]).unwrap());
    }
}
