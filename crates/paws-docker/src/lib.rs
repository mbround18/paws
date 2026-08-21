//! Rust port of `gh-reusable`'s docker-facts/docker-release logic.
//!
//! Parity source (read directly): `packages/dagger-pipelines/src/docker-parity.ts`
//! (`resolveDockerParity`, `findDockerCompose`, `parseDockerCompose`, `shouldPushImage`,
//! `generateTags`). See specs/001-paws-core-cli/spec.md FR-004 and FR-012 for the resolved
//! contract this crate exists to satisfy.
//!
//! Path resolution here is intentionally simpler than the TS source's absolute/relative
//! juggling (`resolvePath`'s `toRelative` dance) — this crate always works with paths
//! relative to the workspace root, which is enough to reproduce every documented behavior
//! (compose discovery, service selection, tag/push decisions) without needing to match the
//! TS source's exact string formatting for absolute paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const DEFAULT_DOCKERFILE: &str = "./Dockerfile";
const DEFAULT_CONTEXT: &str = ".";
const DEFAULT_CANARY_LABEL: &str = "canary";

const COMPOSE_PATHS: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
];

/// Ported from `findDockerCompose`: workspace root first, then the resolved
/// context directory if it differs from the workspace root.
pub fn find_docker_compose(workspace: &Path, context_path: &str) -> Option<PathBuf> {
    for name in COMPOSE_PATHS {
        let candidate = workspace.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if context_path != "." && context_path != "./" {
        let context_dir = workspace.join(context_path);
        if context_dir != workspace {
            for name in COMPOSE_PATHS {
                let candidate = context_dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

#[derive(Debug, Deserialize)]
struct ComposeFile {
    #[serde(default)]
    services: HashMap<String, ComposeService>,
}

#[derive(Debug, Deserialize)]
struct ComposeService {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    build: Option<ComposeBuildField>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ComposeBuildField {
    Context(String),
    Record {
        #[serde(default)]
        dockerfile: Option<String>,
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        args: Option<ComposeBuildArgs>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ComposeBuildArgs {
    Map(HashMap<String, serde_yaml::Value>),
    List(Vec<String>),
}

/// Result of resolving a matched compose service's `build:` field, per
/// `ParseDockerComposeResult`. All-`None`/empty is the documented "no match /
/// no build:" fallback — never an error and never an arbitrary service pick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposeResolution {
    pub dockerfile: Option<String>,
    pub context: Option<String>,
    pub target: Option<String>,
    pub build_args: Vec<(String, String)>,
}

fn parse_build_args(args: Option<ComposeBuildArgs>) -> Vec<(String, String)> {
    match args {
        None => vec![],
        Some(ComposeBuildArgs::Map(map)) => map
            .into_iter()
            .map(|(k, v)| {
                let value = match v {
                    serde_yaml::Value::String(s) => s,
                    other => serde_yaml::to_string(&other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                (k, value)
            })
            .collect(),
        Some(ComposeBuildArgs::List(list)) => list
            .into_iter()
            .filter_map(|entry| {
                let (name, value) = entry.split_once('=')?;
                Some((name.to_string(), value.to_string()))
            })
            .collect(),
    }
}

/// Ported from `parseDockerCompose`. `imageName` matching, service iteration
/// order, and the "first matching service wins, no fallback pick" rule
/// (FR-012 items 2-3) are all load-bearing — a fixture test with two services
/// (one matching, one not) exercises exactly this.
pub fn parse_docker_compose(compose_path: &Path, image_name: &str) -> ComposeResolution {
    let raw = match std::fs::read_to_string(compose_path) {
        Ok(raw) => raw,
        Err(_) => return ComposeResolution::default(),
    };
    // serde_yaml::Mapping preserves document order, matching the TS source's
    // `Object.values(compose.services)` iteration over insertion order.
    let ordered_names: Vec<String> = match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
        Ok(serde_yaml::Value::Mapping(top)) => top
            .get("services")
            .and_then(|v| v.as_mapping())
            .map(|services| {
                services
                    .keys()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        _ => return ComposeResolution::default(),
    };

    let compose: ComposeFile = match serde_yaml::from_str(&raw) {
        Ok(compose) => compose,
        Err(_) => return ComposeResolution::default(),
    };

    let prefix = format!("{image_name}:");
    for name in ordered_names {
        let Some(service) = compose.services.get(&name) else {
            continue;
        };
        let matches = service
            .image
            .as_deref()
            .is_some_and(|image| image.starts_with(&prefix));
        if !matches {
            continue;
        }

        return match &service.build {
            None => ComposeResolution::default(),
            Some(ComposeBuildField::Context(context)) => ComposeResolution {
                context: Some(context.clone()),
                ..Default::default()
            },
            Some(ComposeBuildField::Record {
                dockerfile,
                context,
                target,
                args,
            }) => ComposeResolution {
                dockerfile: dockerfile.clone(),
                context: context.clone(),
                target: target.clone(),
                build_args: parse_build_args(args.clone()),
            },
        };
    }

    ComposeResolution::default()
}

/// Inputs mirroring `DockerParityInputs`.
#[derive(Debug, Clone, Default)]
pub struct DockerFactsInput {
    pub image: String,
    pub version: String,
    pub registries: Vec<String>,
    pub dockerfile: Option<String>,
    pub context: Option<String>,
    pub canary_label: Option<String>,
    pub force_push: bool,
    pub with_latest: bool,
    pub target: Option<String>,
    pub prepend_target: bool,
}

/// Inputs mirroring `DockerParityGithubContext`, minus `eventPath` — callers
/// pass already-extracted PR labels instead of a raw event-file path.
#[derive(Debug, Clone, Default)]
pub struct GithubContext {
    pub workspace: PathBuf,
    pub event_name: String,
    pub git_ref: String,
    pub default_branch: String,
    pub pr_labels: Vec<String>,
}

/// Ported from `shouldPushImage`.
pub fn should_push_image(
    event_name: &str,
    git_ref: &str,
    default_branch: &str,
    canary_label: &str,
    force_push: bool,
    pr_labels: &[String],
) -> bool {
    if force_push {
        return true;
    }
    if git_ref == format!("refs/heads/{default_branch}") {
        return true;
    }
    if git_ref.starts_with("refs/tags/") {
        return true;
    }
    if event_name == "pull_request" {
        return pr_labels.iter().any(|l| l == canary_label);
    }
    false
}

fn is_prerelease_version(version: &str) -> bool {
    ["alpha", "beta", "rc", "dev"]
        .iter()
        .any(|marker| version.contains(marker))
}

fn strip_registry(image: &str) -> String {
    if !image.contains('/') {
        return image.to_string();
    }
    let mut parts = image.splitn(2, '/');
    let first = parts.next().unwrap_or_default();
    let rest = parts.next();
    match rest {
        Some(rest)
            if first.contains('.')
                || first == "localhost"
                || first == "ghcr"
                || first == "docker" =>
        {
            rest.to_string()
        }
        _ => image.to_string(),
    }
}

/// Whether `version` looks like a git commit sha (short or full, hex-only)
/// rather than a semver/tag string — used to pick `sha-`over `v` as the tag
/// prefix. `--version`'s fallback is `$GITHUB_SHA` (see `run_docker`), so an
/// untagged build (a push to a branch, no `--version` override) lands here
/// with a bare hex sha instead of a real version.
fn is_git_sha(version: &str) -> bool {
    (7..=40).contains(&version.len()) && version.chars().all(|c| c.is_ascii_hexdigit())
}

/// Ported from `generateTags`.
pub fn generate_tags(
    image: &str,
    version: &str,
    registries: &[String],
    with_latest: bool,
    git_ref: &str,
    target: &str,
    prepend_target: bool,
) -> Vec<String> {
    let registries: Vec<&String> = registries.iter().filter(|r| !r.is_empty()).collect();
    let target_prefix = if prepend_target && !target.is_empty() {
        format!("{target}-")
    } else {
        String::new()
    };

    let version_tag = if version.starts_with('v') {
        format!("{target_prefix}{version}")
    } else if is_git_sha(version) {
        format!("{target_prefix}sha-{version}")
    } else {
        format!("{target_prefix}v{version}")
    };

    let mut base_tags = vec![format!("{image}:{version_tag}")];
    let is_release_version = git_ref.starts_with("refs/tags/") && !is_prerelease_version(version);
    if with_latest && is_release_version {
        base_tags.push(format!("{image}:{target_prefix}latest"));
    }

    let image_without_registry = strip_registry(image);
    let mut output = Vec::new();
    for base_tag in &base_tags {
        output.push(base_tag.clone());
        let tag_value = base_tag.split(':').nth(1).unwrap_or("latest");
        for registry in &registries {
            output.push(format!("{registry}/{image_without_registry}:{tag_value}"));
        }
    }
    output
}

/// Resolved build facts, mirroring `DockerParityResult` (minus the absolute-path fields —
/// see this module's doc comment on the path-resolution scope reduction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerFacts {
    pub context: String,
    pub dockerfile: String,
    pub target: String,
    pub push: bool,
    pub tags: Vec<String>,
    pub build_args: Vec<(String, String)>,
}

fn join_relative(base: &str, addition: &str) -> String {
    if addition.is_empty() {
        return base.to_string();
    }
    let base_part = base.trim_end_matches('/');
    let base_part = if base_part == "." {
        ""
    } else {
        base_part.trim_start_matches("./")
    };
    let addition_part = addition.trim_start_matches("./");

    if base_part.is_empty() {
        format!("./{addition_part}")
    } else {
        format!("./{base_part}/{addition_part}")
    }
}

/// Ported from `resolveDockerParity`, tying compose discovery/resolution, push
/// gating, and tag generation together (spec.md User Story 3).
pub fn resolve_docker_facts(input: &DockerFactsInput, github: &GithubContext) -> DockerFacts {
    let dockerfile_input = input
        .dockerfile
        .clone()
        .unwrap_or_else(|| DEFAULT_DOCKERFILE.to_string());
    let context_input = input
        .context
        .clone()
        .unwrap_or_else(|| DEFAULT_CONTEXT.to_string());

    let mut context = context_input.clone();
    let mut dockerfile = dockerfile_input.clone();
    let mut target = input.target.clone().unwrap_or_default();
    let mut build_args = Vec::new();

    if let Some(compose_path) = find_docker_compose(&github.workspace, &context_input) {
        let resolution = parse_docker_compose(&compose_path, &input.image);

        match (&resolution.dockerfile, &resolution.context) {
            (Some(df), Some(ctx)) => {
                context = join_relative(&context_input, ctx);
                dockerfile = join_relative(&context, df);
            }
            (Some(df), None) => {
                dockerfile = join_relative(&context_input, df);
            }
            (None, Some(ctx)) => {
                context = join_relative(&context_input, ctx);
            }
            (None, None) => {}
        }

        if let Some(t) = &resolution.target
            && target.is_empty()
        {
            target = t.clone();
        }
        build_args = resolution.build_args;
    }

    let push = should_push_image(
        &github.event_name,
        &github.git_ref,
        &github.default_branch,
        input
            .canary_label
            .as_deref()
            .unwrap_or(DEFAULT_CANARY_LABEL),
        input.force_push,
        &github.pr_labels,
    );

    let tags = generate_tags(
        &input.image,
        &input.version,
        &input.registries,
        input.with_latest,
        &github.git_ref,
        &target,
        input.prepend_target,
    );

    DockerFacts {
        context,
        dockerfile,
        target,
        push,
        tags,
        build_args,
    }
}

/// Registries `dockerRelease` (`gh-reusable`'s, read directly from
/// `packages/dagger-module/src/index.ts`) knows how to authenticate —
/// `docker.io`/`ghcr.io` credentials are hardcoded there, with no generic
/// registry+credential path at all. Anything else in `--registries` (an
/// Artifactory instance, a private registry, ...) gets a tag computed by
/// [`generate_tags`] same as any other registry, but needs a genuinely
/// different publish path — see [`native_publish_pipeline_args`].
pub const KNOWN_DOCKER_RELEASE_REGISTRIES: &[&str] = &["docker.io", "ghcr.io"];

/// Registries in `registries` that `dockerRelease` can't authenticate to
/// itself, in the order given — see [`KNOWN_DOCKER_RELEASE_REGISTRIES`].
pub fn native_registries(registries: &[String]) -> Vec<&str> {
    registries
        .iter()
        .map(|r| r.as_str())
        .filter(|r| !KNOWN_DOCKER_RELEASE_REGISTRIES.contains(r))
        .collect()
}

/// Derives the env var a generic registry's token/password is read from:
/// uppercased, every non-alphanumeric character replaced with `_`, suffixed
/// `_TOKEN` — e.g. `"myco.jfrog.io"` -> `"MYCO_JFROG_IO_TOKEN"`. Mirrors the
/// fixed `DOCKER_TOKEN`/`GHCR_TOKEN` convention `dockerRelease` already uses
/// for its two hardcoded registries, generalized to any registry name.
pub fn registry_token_env_var(registry: &str) -> String {
    let sanitized: String = registry
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}_TOKEN", sanitized.to_ascii_uppercase())
}

/// The subset of `tags` (as produced by [`generate_tags`], already full
/// `registry/image:tag` addresses) that belong to `registry`.
pub fn tags_for_registry<'a>(tags: &'a [String], registry: &str) -> Vec<&'a str> {
    let prefix = format!("{registry}/");
    tags.iter()
        .map(|t| t.as_str())
        .filter(|t| t.starts_with(&prefix))
        .collect()
}

/// The subset of `tags` that are docker.io's — [`generate_tags`] never
/// prefixes docker.io's own tags with a registry hostname at all (a bare
/// `"image:tag"`, or `"org/image:tag"` for a namespaced Docker Hub image —
/// both are valid, unprefixed Docker Hub references, same as `docker push`
/// itself accepts), so these can't be picked out by a `"docker.io/"`
/// prefix the way [`tags_for_registry`] does for every other registry.
/// Identified by elimination instead: whatever isn't prefixed by one of
/// `extra_registries` (`--registries`, i.e. every registry *other* than
/// docker.io) must be a docker.io tag.
pub fn docker_hub_tags<'a>(tags: &'a [String], extra_registries: &[String]) -> Vec<&'a str> {
    tags.iter()
        .map(|t| t.as_str())
        .filter(|t| {
            !extra_registries
                .iter()
                .any(|r| t.starts_with(&format!("{r}/")))
        })
        .collect()
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that builds `context`/`dockerfile`[/`target`] and publishes the result
/// directly to `tag_address` on `registry`, authenticated via
/// `Container.withRegistryAuth` (Dagger's own primitive — bypasses
/// `dockerRelease` entirely, which has no way to authenticate to a
/// registry it doesn't already hardcode). `token_env_var` is read from the
/// *calling* environment via Dagger's `env:NAME` secret-reference syntax —
/// verified for real that `--secret=env:VAR_NAME` resolves correctly
/// against a live Dagger engine, not just documented behavior. One call
/// publishes exactly one tag — `dagger core` chains are strictly linear
/// (`Container.publish` returns the published address, not a `Container`,
/// so nothing can chain after it), so a second tag needs a second
/// invocation of this function; both share this same build prefix, so
/// Dagger's own engine-level caching means the second invocation's build
/// replays from cache rather than rebuilding (same reasoning
/// `paws-helm`'s `publish_packages_pipeline_args`/`publish_index_pipeline_args`
/// split relies on).
fn docker_build_pipeline_prefix(build: &BuildSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "host".into(),
        "directory".into(),
        format!("--path={}", build.context),
        "docker-build".into(),
        format!("--dockerfile={}", build.dockerfile),
    ];
    if !build.target.is_empty() {
        args.push(format!("--target={}", build.target));
    }
    if !build.build_args.is_empty() {
        let joined = build
            .build_args
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        args.push(format!("--build-args={joined}"));
    }
    args
}

/// Builds the `dagger core <chain>` argument list that builds `build`
/// without publishing anywhere — `sync` forces Dagger to actually run the
/// build (it's lazy otherwise) and errors if it fails, without needing a
/// registry to publish to. Used for the "validate the Dockerfile still
/// builds" step on a PR/build-only run — `dockerRelease` (the `gh-reusable`
/// call this crate no longer needs) used to always build regardless of
/// whether it was about to publish; this preserves that same behavior
/// without a registry in the loop at all.
pub fn build_only_pipeline_args(build: &BuildSpec) -> Vec<String> {
    let mut args = docker_build_pipeline_prefix(build);
    args.push("sync".into());
    args
}

pub fn native_publish_pipeline_args(
    build: &BuildSpec,
    publish: &NativeRegistryPublish,
) -> Vec<String> {
    let mut args = docker_build_pipeline_prefix(build);
    args.extend([
        "with-registry-auth".into(),
        format!("--address={}", publish.registry),
        format!("--username={}", publish.username),
        format!("--secret=env:{}", publish.token_env_var),
        "publish".into(),
        format!("--address={}", publish.tag_address),
    ]);
    args
}

/// The build inputs [`native_publish_pipeline_args`] needs — a borrowed
/// view over the same fields [`DockerFacts`] already carries, so callers
/// can pass `&facts` fields straight through without repackaging.
#[derive(Debug, Clone, Copy)]
pub struct BuildSpec<'a> {
    pub context: &'a str,
    pub dockerfile: &'a str,
    pub target: &'a str,
    pub build_args: &'a [(String, String)],
}

/// Where and how [`native_publish_pipeline_args`] authenticates and
/// publishes — one call publishes exactly `tag_address` to `registry`; see
/// that function's doc comment for why one call is one tag.
#[derive(Debug, Clone, Copy)]
pub struct NativeRegistryPublish<'a> {
    pub registry: &'a str,
    pub username: &'a str,
    pub token_env_var: &'a str,
    pub tag_address: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_dir() -> PathBuf {
        // crates/paws-docker -> repo root -> examples
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
    }

    fn base_input(image: &str) -> DockerFactsInput {
        DockerFactsInput {
            image: image.to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        }
    }

    fn base_github(workspace: PathBuf) -> GithubContext {
        GithubContext {
            workspace,
            event_name: "push".to_string(),
            git_ref: "refs/heads/feature".to_string(),
            default_branch: "main".to_string(),
            pr_labels: vec![],
        }
    }

    #[test]
    fn compose_defined_build_resolves_matching_service() {
        let workspace = fixtures_dir().join("docker-compose-fixture");
        let facts =
            resolve_docker_facts(&base_input("ghcr.io/example/app"), &base_github(workspace));

        assert_eq!(facts.context, "./app");
        assert_eq!(facts.dockerfile, "./app/Dockerfile");
        assert_eq!(facts.target, "runtime");
        assert_eq!(
            facts.build_args,
            vec![("FOO".to_string(), "bar".to_string())]
        );
    }

    #[test]
    fn multi_service_compose_picks_first_matching_not_first_in_file() {
        let compose = fixtures_dir().join("docker-compose-fixture/docker-compose.yml");

        // "sidecar" is first in file order but doesn't match; "app" matches
        // and must win. An unmatched image name gets the empty fallback.
        let matched = parse_docker_compose(&compose, "ghcr.io/example/app");
        assert_eq!(matched.context.as_deref(), Some("./app"));

        let unmatched = parse_docker_compose(&compose, "ghcr.io/example/nonexistent");
        assert_eq!(unmatched, ComposeResolution::default());
    }

    #[test]
    fn no_compose_file_falls_back_to_plain_dockerfile_and_dot_context() {
        let workspace = fixtures_dir().join("docker-fixture");
        let facts = resolve_docker_facts(
            &base_input("ghcr.io/example/plain"),
            &base_github(workspace),
        );

        assert_eq!(facts.context, ".");
        assert_eq!(facts.dockerfile, "./Dockerfile");
    }

    #[test]
    fn canary_label_gates_push_on_pull_requests() {
        let workspace = fixtures_dir().join("docker-fixture");
        let mut github = base_github(workspace);
        github.event_name = "pull_request".to_string();
        github.git_ref = "refs/heads/some-pr-branch".to_string();

        let without_label = resolve_docker_facts(&base_input("img"), &github);
        assert!(!without_label.push);

        github.pr_labels = vec!["canary".to_string()];
        let with_label = resolve_docker_facts(&base_input("img"), &github);
        assert!(with_label.push);
    }

    #[test]
    fn force_push_overrides_every_other_gate() {
        let workspace = fixtures_dir().join("docker-fixture");
        let github = base_github(workspace);
        let mut input = base_input("img");
        input.force_push = true;

        let facts = resolve_docker_facts(&input, &github);
        assert!(facts.push);
    }

    #[test]
    fn default_branch_and_tag_refs_always_push() {
        let workspace = fixtures_dir().join("docker-fixture");
        let mut github = base_github(workspace);
        github.git_ref = "refs/heads/main".to_string();
        assert!(resolve_docker_facts(&base_input("img"), &github).push);

        github.git_ref = "refs/tags/v1.0.0".to_string();
        assert!(resolve_docker_facts(&base_input("img"), &github).push);
    }

    #[test]
    fn generate_tags_expands_across_registries_and_latest() {
        let tags = generate_tags(
            "ghcr.io/example/app",
            "1.2.3",
            &["docker.io/mirror".to_string()],
            true,
            "refs/tags/v1.2.3",
            "",
            false,
        );

        assert_eq!(
            tags,
            vec![
                "ghcr.io/example/app:v1.2.3".to_string(),
                "docker.io/mirror/example/app:v1.2.3".to_string(),
                "ghcr.io/example/app:latest".to_string(),
                "docker.io/mirror/example/app:latest".to_string(),
            ]
        );
    }

    #[test]
    fn git_sha_versions_use_a_sha_prefix_not_v() {
        let tags = generate_tags("app", "e4a17f4", &[], true, "refs/heads/main", "", false);
        assert_eq!(tags, vec!["app:sha-e4a17f4".to_string()]);

        let tags = generate_tags(
            "app",
            "e4a17f4d4e0f1b10182564dd7beb9515017184bf",
            &[],
            true,
            "refs/heads/main",
            "",
            false,
        );
        assert_eq!(
            tags,
            vec!["app:sha-e4a17f4d4e0f1b10182564dd7beb9515017184bf".to_string()]
        );
    }

    #[test]
    fn prerelease_versions_never_get_a_latest_tag() {
        let tags = generate_tags(
            "app",
            "1.2.3-rc.1",
            &[],
            true,
            "refs/tags/v1.2.3-rc.1",
            "",
            false,
        );
        assert_eq!(tags, vec!["app:v1.2.3-rc.1".to_string()]);
    }

    #[test]
    fn native_registries_excludes_docker_hub_and_ghcr() {
        let registries = vec![
            "docker.io".to_string(),
            "ghcr.io".to_string(),
            "myco.jfrog.io".to_string(),
        ];
        assert_eq!(native_registries(&registries), vec!["myco.jfrog.io"]);
    }

    #[test]
    fn native_registries_is_empty_for_only_known_registries() {
        let registries = vec!["docker.io".to_string(), "ghcr.io".to_string()];
        assert!(native_registries(&registries).is_empty());
    }

    #[test]
    fn registry_token_env_var_sanitizes_the_registry_name() {
        assert_eq!(
            registry_token_env_var("myco.jfrog.io"),
            "MYCO_JFROG_IO_TOKEN"
        );
        assert_eq!(
            registry_token_env_var("registry.example.com:5000"),
            "REGISTRY_EXAMPLE_COM_5000_TOKEN"
        );
    }

    #[test]
    fn tags_for_registry_filters_to_matching_prefix_only() {
        let tags = vec![
            "app:v1.0.0".to_string(),
            "ghcr.io/app:v1.0.0".to_string(),
            "myco.jfrog.io/app:v1.0.0".to_string(),
            "myco.jfrog.io/app:latest".to_string(),
        ];
        assert_eq!(
            tags_for_registry(&tags, "myco.jfrog.io"),
            vec!["myco.jfrog.io/app:v1.0.0", "myco.jfrog.io/app:latest"]
        );
    }

    #[test]
    fn docker_hub_tags_finds_unprefixed_tags_by_elimination() {
        let tags = vec![
            "app:v1.0.0".to_string(),
            "ghcr.io/app:v1.0.0".to_string(),
            "myco.jfrog.io/app:v1.0.0".to_string(),
        ];
        assert_eq!(
            docker_hub_tags(&tags, &["ghcr.io".to_string(), "myco.jfrog.io".to_string()]),
            vec!["app:v1.0.0"]
        );
    }

    #[test]
    fn docker_hub_tags_handles_a_namespaced_docker_hub_image() {
        // "mbround18/steamcmd:base-v0.1.0" has a "/" in it too, but it's
        // still a bare (docker.io) reference, not a registry-prefixed one -
        // elimination against known extra registries must not mistake the
        // org/repo separator for a registry hostname.
        let tags = vec![
            "mbround18/steamcmd:base-v0.1.0".to_string(),
            "ghcr.io/mbround18/steamcmd:base-v0.1.0".to_string(),
        ];
        assert_eq!(
            docker_hub_tags(&tags, &["ghcr.io".to_string()]),
            vec!["mbround18/steamcmd:base-v0.1.0"]
        );
    }

    #[test]
    fn build_only_pipeline_args_builds_without_a_registry() {
        let args = build_only_pipeline_args(&BuildSpec {
            context: ".",
            dockerfile: "./Dockerfile",
            target: "base",
            build_args: &[],
        });
        assert_eq!(
            args,
            vec![
                "host".to_string(),
                "directory".to_string(),
                "--path=.".to_string(),
                "docker-build".to_string(),
                "--dockerfile=./Dockerfile".to_string(),
                "--target=base".to_string(),
                "sync".to_string(),
            ]
        );
    }

    #[test]
    fn native_publish_pipeline_args_builds_and_publishes_one_tag() {
        let build_args = vec![("UBUNTU_VERSION".to_string(), "24.04".to_string())];
        let args = native_publish_pipeline_args(
            &BuildSpec {
                context: ".",
                dockerfile: "./Dockerfile",
                target: "base",
                build_args: &build_args,
            },
            &NativeRegistryPublish {
                registry: "myco.jfrog.io",
                username: "deploy-bot",
                token_env_var: "MYCO_JFROG_IO_TOKEN",
                tag_address: "myco.jfrog.io/steamcmd:base-v0.1.0",
            },
        );

        assert_eq!(
            args,
            vec![
                "host".to_string(),
                "directory".to_string(),
                "--path=.".to_string(),
                "docker-build".to_string(),
                "--dockerfile=./Dockerfile".to_string(),
                "--target=base".to_string(),
                "--build-args=UBUNTU_VERSION=24.04".to_string(),
                "with-registry-auth".to_string(),
                "--address=myco.jfrog.io".to_string(),
                "--username=deploy-bot".to_string(),
                "--secret=env:MYCO_JFROG_IO_TOKEN".to_string(),
                "publish".to_string(),
                "--address=myco.jfrog.io/steamcmd:base-v0.1.0".to_string(),
            ]
        );
    }

    #[test]
    fn native_publish_pipeline_args_omits_target_and_build_args_when_empty() {
        let args = native_publish_pipeline_args(
            &BuildSpec {
                context: ".",
                dockerfile: "./Dockerfile",
                target: "",
                build_args: &[],
            },
            &NativeRegistryPublish {
                registry: "myco.jfrog.io",
                username: "deploy-bot",
                token_env_var: "MYCO_JFROG_IO_TOKEN",
                tag_address: "myco.jfrog.io/steamcmd:v0.1.0",
            },
        );
        assert!(!args.iter().any(|a| a.starts_with("--target=")));
        assert!(!args.iter().any(|a| a.starts_with("--build-args=")));
    }
}
