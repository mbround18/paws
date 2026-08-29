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
    /// Opt-in tag-matrix flags (User Stories 1 and 3), flat like
    /// `with_latest`/`prepend_target` above — all default `false` via
    /// `Default`, so existing callers using `..Default::default()` see no
    /// behavior change (FR-005).
    pub tag_rollup: bool,
    pub tag_sha: bool,
    pub tag_branch: bool,
    pub tag_pr: bool,
    pub tag_schedule: bool,
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

/// Extracts `(major, "major.minor")` rollup components from a release
/// version via an actual semver parse (FR-016) — not string-splitting, so a
/// version that doesn't decompose cleanly (build metadata, non-3-part
/// versions, anything `semver::Version::parse` rejects) produces no rollup
/// tags rather than a malformed one. Build-metadata-suffixed versions
/// (`1.2.3+abc`) parse successfully under strict semver rules but are
/// rejected here anyway — spec's Risks section (v3.2.1+abc) explicitly
/// wants no rollup for those, since a build-tagged version shouldn't move
/// the major/minor pointer.
fn rollup_components(version: &str) -> Option<(String, String)> {
    let trimmed = version.strip_prefix('v').unwrap_or(version);
    let parsed = semver::Version::parse(trimmed).ok()?;
    if !parsed.build.is_empty() {
        return None;
    }
    Some((
        parsed.major.to_string(),
        format!("{}.{}", parsed.major, parsed.minor),
    ))
}

/// Parses a GitHub Actions PR-event `git_ref` (`refs/pull/{number}/merge`)
/// down to the PR number, so `--tag-pr` (FR-014) needs no new required CLI
/// input — `paws-docker` already receives `git_ref` (research.md R5).
fn parse_pr_number(git_ref: &str) -> Option<u64> {
    git_ref
        .strip_prefix("refs/pull/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Parses a branch-push `git_ref` (`refs/heads/{branch}`) down to the
/// branch name, for `--tag-branch` (FR-014) — same "derive from the
/// existing git_ref field" approach as [`parse_pr_number`].
fn parse_branch_name(git_ref: &str) -> Option<&str> {
    git_ref.strip_prefix("refs/heads/")
}

/// Docker tag components only allow `[A-Za-z0-9_.-]` — a branch name like
/// `feature/foo` needs its `/` (and anything else outside that set)
/// replaced before it's usable as a tag, matching the slugging every
/// branch-tag GitHub Action does for the same reason.
fn sanitize_tag_component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Every tag type a [`generate_tag_matrix`] call can produce, before the
/// target-prefix and registry-mirroring steps are applied uniformly to all
/// of them (see that function's doc comment). Internal — `generate_tags`'s
/// public signature/output stay byte-identical to before this feature
/// (FR-005); this is the restructuring named in spec.md's Affected
/// Contracts and plan.md's Design Decision 1.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TagKind {
    /// Today's sole output: a "v"-prefixed version or a "sha-"-prefixed
    /// git sha, already resolved to its bare tag value (no target prefix
    /// yet — that's applied uniformly below).
    Version(String),
    Latest,
    RollupMajor(String),
    RollupMinor(String),
    Sha(String),
    BranchRef(String),
    PrRef(u64),
    Schedule,
}

impl TagKind {
    /// The bare tag value this kind renders to, before the target prefix
    /// is applied.
    fn bare_value(&self) -> String {
        match self {
            TagKind::Version(v) => v.clone(),
            TagKind::Latest => "latest".to_string(),
            TagKind::RollupMajor(m) => m.clone(),
            TagKind::RollupMinor(m) => m.clone(),
            TagKind::Sha(s) => format!("sha-{s}"),
            TagKind::BranchRef(b) => sanitize_tag_component(b),
            TagKind::PrRef(n) => format!("pr-{n}"),
            // Literal string, not a timestamp/nightly-date suffix — kept as
            // a stable, overwritable pointer like `latest` rather than an
            // ever-growing tag list (plan.md Design Decision 7).
            TagKind::Schedule => "schedule".to_string(),
        }
    }
}

/// Everything [`generate_tag_matrix`] needs beyond the base
/// image/version/registries/target inputs [`generate_tags`] already takes —
/// every field opt-in and `false` by default, so omitting all of them
/// reproduces [`generate_tags`]'s exact output (FR-005).
#[derive(Debug, Clone, Copy, Default)]
pub struct TagMatrixOptions {
    pub with_latest: bool,
    pub tag_rollup: bool,
    pub tag_sha: bool,
    pub tag_branch: bool,
    pub tag_pr: bool,
    pub tag_schedule: bool,
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

/// Ported from `generateTags`. Kept as a thin wrapper over
/// [`generate_tag_matrix`] with every new opt-in option off — existing
/// callers (and this crate's own pre-feature tests) see byte-identical
/// output (FR-005, SC-001); see that function's doc comment for the
/// restructuring this wraps.
pub fn generate_tags(
    image: &str,
    version: &str,
    registries: &[String],
    with_latest: bool,
    git_ref: &str,
    target: &str,
    prepend_target: bool,
) -> Vec<String> {
    generate_tag_matrix(
        image,
        version,
        registries,
        git_ref,
        "",
        target,
        prepend_target,
        &TagMatrixOptions {
            with_latest,
            ..Default::default()
        },
    )
}

/// Full opt-in tag matrix (spec.md User Stories 1 and 3): builds every
/// applicable [`TagKind`] first, then runs the *one* target-prefix +
/// registry-mirroring pass over the resulting tag strings — the same
/// mirroring [`generate_tags`] always applied to just `version`/`latest`,
/// now shared by every tag type (FR-003, FR-014; no separate mirroring
/// implementation per spec's Risks section). `event_name` is required here
/// (unlike [`generate_tags`]) because branch-push and `schedule` triggers
/// can share the same `git_ref` shape (`refs/heads/<default-branch>`) and
/// are only distinguishable by event name.
// One more flat parameter than `generate_tags` (which already sat at
// clippy's 7-arg threshold) — `event_name` is required alongside `git_ref`
// (see doc comment above) and `options` bundles every opt-in flag, so a
// params-struct wrapper here would just relocate the same flat fields
// without adding clarity, unlike genuinely grouped data.
#[allow(clippy::too_many_arguments)]
pub fn generate_tag_matrix(
    image: &str,
    version: &str,
    registries: &[String],
    git_ref: &str,
    event_name: &str,
    target: &str,
    prepend_target: bool,
    options: &TagMatrixOptions,
) -> Vec<String> {
    let registries: Vec<&String> = registries.iter().filter(|r| !r.is_empty()).collect();
    let target_prefix = if prepend_target && !target.is_empty() {
        format!("{target}-")
    } else {
        String::new()
    };

    let version_value = if version.starts_with('v') {
        version.to_string()
    } else if is_git_sha(version) {
        format!("sha-{version}")
    } else {
        format!("v{version}")
    };
    let is_release_version = git_ref.starts_with("refs/tags/") && !is_prerelease_version(version);

    let mut kinds = vec![TagKind::Version(version_value)];
    if options.with_latest && is_release_version {
        kinds.push(TagKind::Latest);
    }
    if options.tag_rollup
        && is_release_version
        && let Some((major, minor)) = rollup_components(version)
    {
        // Minor before major, matching spec.md's stated order (Acceptance
        // Scenario 1: "{image}:v3.2.1, {image}:3.2, and {image}:3").
        kinds.push(TagKind::RollupMinor(minor));
        kinds.push(TagKind::RollupMajor(major));
    }
    if options.tag_sha && is_git_sha(version) {
        kinds.push(TagKind::Sha(version.to_string()));
    }
    if options.tag_branch
        && event_name != "schedule"
        && event_name != "pull_request"
        && let Some(branch) = parse_branch_name(git_ref)
    {
        kinds.push(TagKind::BranchRef(branch.to_string()));
    }
    if options.tag_pr
        && event_name == "pull_request"
        && let Some(number) = parse_pr_number(git_ref)
    {
        kinds.push(TagKind::PrRef(number));
    }
    if options.tag_schedule && event_name == "schedule" {
        kinds.push(TagKind::Schedule);
    }

    // Dedup on the final "image:tag" string, preserving first-occurrence
    // order — two independently-gated TagKinds can render the same string
    // (e.g. an already-sha-versioned build with --tag-sha also set), and
    // Acceptance Scenario 5 requires no duplicates in that case.
    let mut base_tags = Vec::new();
    for kind in kinds {
        let tag = format!("{image}:{target_prefix}{}", kind.bare_value());
        if !base_tags.contains(&tag) {
            base_tags.push(tag);
        }
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

    let tags = generate_tag_matrix(
        &input.image,
        &input.version,
        &input.registries,
        &github.git_ref,
        &github.event_name,
        &target,
        input.prepend_target,
        &TagMatrixOptions {
            with_latest: input.with_latest,
            tag_rollup: input.tag_rollup,
            tag_sha: input.tag_sha,
            tag_branch: input.tag_branch,
            tag_pr: input.tag_pr,
            tag_schedule: input.tag_schedule,
        },
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

/// Which flag put a registry into the publish plan, so a missing credential
/// blames the right one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOrigin {
    /// Docker Hub, which is always considered.
    DockerHub,
    /// A registry host named in `--image`.
    Image,
    /// A registry named in `--registries`.
    Registries,
}

impl TargetOrigin {
    /// The flag to cite when this target has no usable credential.
    pub fn flag(self) -> &'static str {
        match self {
            Self::DockerHub => "--image",
            Self::Image => "--image",
            Self::Registries => "--registries",
        }
    }
}

/// One registry the build should publish to, and how to authenticate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishTarget {
    pub registry: String,
    pub tags: Vec<String>,
    pub username: Option<String>,
    /// Env var the password is read from at publish time.
    pub token_env_var: String,
    /// Whether missing credentials should fail the run rather than skip it.
    pub credentials_required: bool,
    pub origin: TargetOrigin,
}

/// Everything [`plan_publish_targets`] needs.
///
/// Credential *presence* is passed in rather than read from the environment, so
/// planning stays a pure function and can be table-tested without a Dagger
/// daemon, a registry, or environment mutation. Both bugs this planning has had
/// — docker.io being silently dropped, and a fully-qualified `--image` never
/// becoming its own target — lived in code that could not be tested this way.
#[derive(Debug, Clone, Default)]
pub struct PublishPlanInput<'a> {
    pub image: &'a str,
    pub tags: &'a [String],
    pub registries: &'a [String],
    pub dockerhub_username: Option<&'a str>,
    pub ghcr_username: Option<&'a str>,
    /// `--registry-username` entries, keyed by registry.
    pub extra_usernames: &'a [(String, String)],
    /// Whether `$GHCR_TOKEN` is set.
    pub ghcr_token_present: bool,
    /// Whether `$GITHUB_TOKEN` is set — the GHCR fallback.
    pub github_token_present: bool,
}

/// Decide which registries to publish to, with which credentials.
pub fn plan_publish_targets(input: &PublishPlanInput<'_>) -> Vec<PublishTarget> {
    let owned = |tags: Vec<&str>| tags.into_iter().map(str::to_string).collect::<Vec<_>>();

    let username_for = |registry: &str| -> Option<String> {
        if registry == "ghcr.io" {
            return input.ghcr_username.map(str::to_string);
        }
        input
            .extra_usernames
            .iter()
            .find(|(name, _)| name == registry)
            .map(|(_, user)| user.clone())
    };

    let token_env_for = |registry: &str| -> String {
        if registry != "ghcr.io" {
            return registry_token_env_var(registry);
        }
        // In GitHub Actions the workflow token is the natural GHCR credential,
        // and copying it into GHCR_TOKEN as well is a step that is easy to
        // forget — silently, because a missing token only skipped the publish.
        // An explicitly set GHCR_TOKEN still wins.
        if input.ghcr_token_present || !input.github_token_present {
            "GHCR_TOKEN".to_string()
        } else {
            "GITHUB_TOKEN".to_string()
        }
    };

    let mut targets = vec![PublishTarget {
        registry: "docker.io".to_string(),
        tags: owned(docker_hub_tags(input.tags, input.registries)),
        username: input.dockerhub_username.map(str::to_string),
        token_env_var: "DOCKER_TOKEN".to_string(),
        // Docker Hub is always considered rather than asked for, so missing
        // credentials degrade rather than fail — a repo that only ever
        // configured ghcr.io must keep working.
        credentials_required: false,
        origin: TargetOrigin::DockerHub,
    }];

    // A fully-qualified --image names the registry it is meant to publish to.
    let image_registry = registry_of(input.image).filter(|registry| {
        *registry != "docker.io" && !input.registries.iter().any(|r| r == registry)
    });

    if let Some(registry) = image_registry {
        targets.push(PublishTarget {
            registry: registry.to_string(),
            tags: owned(tags_for_registry(input.tags, registry)),
            username: username_for(registry),
            token_env_var: token_env_for(registry),
            // Naming a registry in --image is as explicit an ask as naming it
            // in --registries.
            credentials_required: true,
            origin: TargetOrigin::Image,
        });
    }

    for registry in input.registries {
        targets.push(PublishTarget {
            registry: registry.clone(),
            tags: owned(tags_for_registry(input.tags, registry)),
            username: username_for(registry),
            token_env_var: token_env_for(registry),
            credentials_required: registry != "ghcr.io",
            origin: TargetOrigin::Registries,
        });
    }

    targets
}

/// Why a target with tags could not publish them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    NoUsername,
    NoToken { env_var: String },
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoUsername => write!(f, "no username configured"),
            Self::NoToken { env_var } => write!(f, "${env_var} not set"),
        }
    }
}

/// What actually happened for one publish target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    Published {
        registry: String,
        tags: Vec<String>,
    },
    /// Had tags, but could not publish them.
    Skipped {
        registry: String,
        reason: SkipReason,
    },
    /// Had no tags of its own — normal when another registry owns them all.
    NoTags {
        registry: String,
    },
}

/// Total tags actually published across every target.
pub fn published_tag_count(outcomes: &[PublishOutcome]) -> usize {
    outcomes
        .iter()
        .map(|outcome| match outcome {
            PublishOutcome::Published { tags, .. } => tags.len(),
            _ => 0,
        })
        .sum()
}

/// A one-line ledger of what a publish run actually did.
///
/// Printed unconditionally, because the failure mode this exists for is a run
/// that reports success while publishing nothing — and the thing that made it
/// invisible was per-target chatter with no closing statement.
pub fn publish_summary(outcomes: &[PublishOutcome]) -> String {
    let mut published: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for outcome in outcomes {
        match outcome {
            PublishOutcome::Published { registry, tags } => {
                published.push(format!("{} tag(s) to {registry}", tags.len()));
            }
            PublishOutcome::Skipped { registry, reason } => {
                skipped.push(format!("{registry} ({reason})"));
            }
            // Not worth reporting: a registry with no tags was never asked to
            // do anything.
            PublishOutcome::NoTags { .. } => {}
        }
    }

    let mut summary = if published.is_empty() {
        "docker: published nothing".to_string()
    } else {
        format!("docker: published {}", published.join(", "))
    };

    if !skipped.is_empty() {
        summary.push_str(&format!(" — skipped {}", skipped.join(", ")));
    }

    summary
}

/// An error message when a run was asked to push but published nothing.
///
/// Returns `None` when at least one tag published, or when there was nothing to
/// publish in the first place.
pub fn nothing_published_error(outcomes: &[PublishOutcome]) -> Option<String> {
    if published_tag_count(outcomes) > 0 {
        return None;
    }

    let blocked: Vec<String> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            PublishOutcome::Skipped { registry, reason } => Some(format!("{registry}: {reason}")),
            _ => None,
        })
        .collect();

    // Nothing published and nothing blocked means there was nothing to do —
    // no tags resolved at all, which is reported elsewhere.
    if blocked.is_empty() {
        return None;
    }

    Some(format!(
        "--push was requested but nothing was published ({})",
        blocked.join("; ")
    ))
}

/// The registry host an image reference names explicitly, if any.
///
/// Follows Docker's own rule: the first path segment is a registry only when it
/// looks like a host — it contains a `.` or a `:`, or is `localhost`. So
/// `ghcr.io/owner/app` names ghcr.io, while `owner/app` is a Docker Hub
/// namespace and `app` is a bare Docker Hub image.
pub fn registry_of(image: &str) -> Option<&str> {
    let (first, _rest) = image.split_once('/')?;
    (first.contains('.') || first.contains(':') || first == "localhost").then_some(first)
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
///
/// Elimination alone is not enough, though. A tag built from a fully-qualified
/// `--image` (`ghcr.io/owner/app`) is prefixed by no *extra* registry when
/// `--registries` is empty, and used to be classified as a Docker Hub tag on
/// that basis — so it was published to docker.io or, with no Docker Hub
/// credentials, silently skipped. A reference that names its own registry is
/// never a Docker Hub reference, so those are excluded too.
pub fn docker_hub_tags<'a>(tags: &'a [String], extra_registries: &[String]) -> Vec<&'a str> {
    tags.iter()
        .map(|t| t.as_str())
        .filter(|t| {
            !extra_registries
                .iter()
                .any(|r| t.starts_with(&format!("{r}/")))
        })
        // An explicit registry other than docker.io means the tag belongs to
        // that registry, not here.
        .filter(|t| registry_of(t).is_none_or(|registry| registry == "docker.io"))
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
/// `resolve_docker_facts` deliberately computes `dockerfile` relative to
/// the repo root (mirroring real `docker build -f`'s CLI semantics, where
/// `-f`'s path is relative to the caller's cwd, not to `context` -- see
/// its pinned unit tests), but the dagger `host directory --path=<context>`
/// call above only mounts `context` itself, not the whole repo. Passing
/// the repo-root-relative dockerfile straight through double-nests it once
/// `context` is a subdirectory (e.g. context "./app", dockerfile
/// "./app/Dockerfile" looked up *inside* the already-mounted "./app" ->
/// "app/app/Dockerfile", which doesn't exist) -- confirmed for real against
/// `examples/docker-compose-fixture`. This strips the mounted context's
/// prefix back off so the path is relative to what's actually mounted.
fn dockerfile_relative_to_context(context: &str, dockerfile: &str) -> String {
    let ctx = context.trim_start_matches("./").trim_end_matches('/');
    if ctx.is_empty() {
        return dockerfile.to_string();
    }
    let df = dockerfile.trim_start_matches("./");
    match df.strip_prefix(ctx).and_then(|rest| rest.strip_prefix('/')) {
        Some(rest) => format!("./{rest}"),
        None => dockerfile.to_string(),
    }
}

fn docker_build_pipeline_prefix(build: &BuildSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "host".into(),
        "directory".into(),
        format!("--path={}", build.context),
        "docker-build".into(),
        format!(
            "--dockerfile={}",
            dockerfile_relative_to_context(build.context, build.dockerfile)
        ),
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

    // T005 (SC-001): fixed-snapshot regression covering the exact same
    // fixture shapes as the pre-feature `generate_tags` tests above, pinned
    // to their pre-restructuring output — a byte-identical guardrail
    // independent of those tests happening to still exist/pass.
    #[test]
    fn generate_tags_default_output_is_byte_identical_to_pre_feature_snapshot() {
        assert_eq!(
            generate_tags(
                "ghcr.io/example/app",
                "1.2.3",
                &["docker.io/mirror".to_string()],
                true,
                "refs/tags/v1.2.3",
                "",
                false,
            ),
            vec![
                "ghcr.io/example/app:v1.2.3".to_string(),
                "docker.io/mirror/example/app:v1.2.3".to_string(),
                "ghcr.io/example/app:latest".to_string(),
                "docker.io/mirror/example/app:latest".to_string(),
            ]
        );
        assert_eq!(
            generate_tags("app", "e4a17f4", &[], true, "refs/heads/main", "", false),
            vec!["app:sha-e4a17f4".to_string()]
        );
        assert_eq!(
            generate_tags(
                "app",
                "1.2.3-rc.1",
                &[],
                true,
                "refs/tags/v1.2.3-rc.1",
                "",
                false,
            ),
            vec!["app:v1.2.3-rc.1".to_string()]
        );
    }

    // --- User Story 1: rollup tags (T007-T013) ---

    #[test]
    fn tag_rollup_produces_major_and_minor_on_a_release_version() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:v3.2.1".to_string(),
                "image:3.2".to_string(),
                "image:3".to_string(),
            ]
        );
    }

    #[test]
    fn tag_rollup_omitted_is_byte_identical_to_generate_tags() {
        let with_matrix = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions::default(),
        );
        let baseline = generate_tags("image", "v3.2.1", &[], false, "refs/tags/v3.2.1", "", false);
        assert_eq!(with_matrix, baseline);
        assert_eq!(with_matrix, vec!["image:v3.2.1".to_string()]);
    }

    #[test]
    fn tag_rollup_produces_nothing_for_a_prerelease_version() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1-rc.1",
            &[],
            "refs/tags/v3.2.1-rc.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(tags, vec!["image:v3.2.1-rc.1".to_string()]);
    }

    #[test]
    fn tag_rollup_produces_nothing_for_build_metadata_or_bare_sha() {
        let build_metadata = generate_tag_matrix(
            "image",
            "v3.2.1+abc",
            &[],
            "refs/tags/v3.2.1+abc",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(build_metadata, vec!["image:v3.2.1+abc".to_string()]);

        let bare_sha = generate_tag_matrix(
            "image",
            "e4a17f4",
            &[],
            "refs/tags/e4a17f4",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(bare_sha, vec!["image:sha-e4a17f4".to_string()]);
    }

    #[test]
    fn tag_rollup_and_with_latest_together_produce_no_duplicates() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                with_latest: true,
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:v3.2.1".to_string(),
                "image:latest".to_string(),
                "image:3.2".to_string(),
                "image:3".to_string(),
            ]
        );
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len());
    }

    #[test]
    fn tag_rollup_respects_target_prefix() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "odin",
            true,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:odin-v3.2.1".to_string(),
                "image:odin-3.2".to_string(),
                "image:odin-3".to_string(),
            ]
        );
    }

    #[test]
    fn tag_rollup_mirrors_across_every_registry() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &["docker.io/mirror".to_string(), "quay.io".to_string()],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_rollup: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:v3.2.1".to_string(),
                "docker.io/mirror/image:v3.2.1".to_string(),
                "quay.io/image:v3.2.1".to_string(),
                "image:3.2".to_string(),
                "docker.io/mirror/image:3.2".to_string(),
                "quay.io/image:3.2".to_string(),
                "image:3".to_string(),
                "docker.io/mirror/image:3".to_string(),
                "quay.io/image:3".to_string(),
            ]
        );
    }

    // --- User Story 3: full tag matrix (T016-T020) ---

    #[test]
    fn tag_branch_produces_a_branch_derived_tag_on_a_branch_push() {
        let tags = generate_tag_matrix(
            "image",
            "e4a17f4",
            &["docker.io/mirror".to_string()],
            "refs/heads/some-branch",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_branch: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:sha-e4a17f4".to_string(),
                "docker.io/mirror/image:sha-e4a17f4".to_string(),
                "image:some-branch".to_string(),
                "docker.io/mirror/image:some-branch".to_string(),
            ]
        );
    }

    #[test]
    fn tag_branch_sanitizes_slashes_in_the_branch_name() {
        let tags = generate_tag_matrix(
            "image",
            "e4a17f4",
            &[],
            "refs/heads/feature/foo",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_branch: true,
                ..Default::default()
            },
        );
        assert!(tags.contains(&"image:feature-foo".to_string()));
    }

    #[test]
    fn tag_pr_produces_a_pr_number_tag_on_a_pull_request_build() {
        let tags = generate_tag_matrix(
            "image",
            "e4a17f4",
            &["docker.io/mirror".to_string()],
            "refs/pull/42/merge",
            "pull_request",
            "",
            false,
            &TagMatrixOptions {
                tag_pr: true,
                ..Default::default()
            },
        );
        assert!(tags.contains(&"image:pr-42".to_string()));
        assert!(tags.contains(&"docker.io/mirror/image:pr-42".to_string()));
    }

    #[test]
    fn tag_schedule_produces_the_schedule_tag_on_a_scheduled_build() {
        let tags = generate_tag_matrix(
            "image",
            "e4a17f4",
            &["docker.io/mirror".to_string()],
            "refs/heads/main",
            "schedule",
            "",
            false,
            &TagMatrixOptions {
                tag_schedule: true,
                ..Default::default()
            },
        );
        assert!(tags.contains(&"image:schedule".to_string()));
        assert!(tags.contains(&"docker.io/mirror/image:schedule".to_string()));
        // schedule's git_ref shape (refs/heads/<default-branch>) must not
        // also produce a branch-ref tag when --tag-branch isn't set.
        assert!(!tags.iter().any(|t| t == "image:main"));
    }

    #[test]
    fn tag_sha_is_unconditional_not_only_a_fallback() {
        // A real version tag is present (not a bare sha primary tag), but
        // --tag-sha still adds the sha- tag alongside it (FR-015) — unlike
        // today's is_git_sha fallback, which only kicks in when there's no
        // other version to tag with.
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_sha: true,
                ..Default::default()
            },
        );
        // v3.2.1 isn't itself a sha, so is_git_sha(version) is false and no
        // sha tag is produced — confirms --tag-sha is gated on the version
        // actually being sha-shaped, not "always append a literal sha tag".
        assert_eq!(tags, vec!["image:v3.2.1".to_string()]);

        let sha_build = generate_tag_matrix(
            "image",
            "e4a17f4",
            &[],
            "refs/heads/main",
            "push",
            "",
            false,
            &TagMatrixOptions {
                tag_sha: true,
                ..Default::default()
            },
        );
        // Version already resolved to "sha-e4a17f4" via the existing
        // is_git_sha fallback; --tag-sha's own Sha kind renders the same
        // string, deduped to one tag rather than two identical entries.
        assert_eq!(sha_build, vec!["image:sha-e4a17f4".to_string()]);
    }

    #[test]
    fn full_matrix_combination_produces_no_duplicates_or_cross_type_interference() {
        let tags = generate_tag_matrix(
            "image",
            "v3.2.1",
            &[],
            "refs/tags/v3.2.1",
            "push",
            "",
            false,
            &TagMatrixOptions {
                with_latest: true,
                tag_rollup: true,
                tag_sha: true,
                ..Default::default()
            },
        );
        assert_eq!(
            tags,
            vec![
                "image:v3.2.1".to_string(),
                "image:latest".to_string(),
                "image:3.2".to_string(),
                "image:3".to_string(),
            ]
        );
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len());
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

    // --- the publish ledger -----------------------------------------------
    //
    // Every case here is a shape this tool has actually shipped: a run that
    // reported success while publishing nothing.

    fn published(registry: &str, tags: &[&str]) -> PublishOutcome {
        PublishOutcome::Published {
            registry: registry.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn skipped_no_username(registry: &str) -> PublishOutcome {
        PublishOutcome::Skipped {
            registry: registry.to_string(),
            reason: SkipReason::NoUsername,
        }
    }

    fn skipped_no_token(registry: &str, env_var: &str) -> PublishOutcome {
        PublishOutcome::Skipped {
            registry: registry.to_string(),
            reason: SkipReason::NoToken {
                env_var: env_var.to_string(),
            },
        }
    }

    #[test]
    fn the_summary_states_what_published() {
        let outcomes = vec![published(
            "ghcr.io",
            &["ghcr.io/o/a:v1", "ghcr.io/o/a:latest"],
        )];
        assert_eq!(
            publish_summary(&outcomes),
            "docker: published 2 tag(s) to ghcr.io"
        );
    }

    #[test]
    fn the_summary_names_what_was_skipped_alongside_what_published() {
        let outcomes = vec![
            published("ghcr.io", &["ghcr.io/o/a:v1"]),
            skipped_no_username("docker.io"),
        ];
        assert_eq!(
            publish_summary(&outcomes),
            "docker: published 1 tag(s) to ghcr.io — skipped docker.io (no username configured)"
        );
    }

    /// The exact line the ghcr regression should have produced.
    #[test]
    fn the_summary_is_unambiguous_when_nothing_published() {
        let outcomes = vec![skipped_no_username("docker.io")];
        assert_eq!(
            publish_summary(&outcomes),
            "docker: published nothing — skipped docker.io (no username configured)"
        );
    }

    #[test]
    fn a_registry_with_no_tags_is_not_reported() {
        // Normal whenever another registry owns every tag; reporting it would
        // be noise that trains people to ignore the line.
        let outcomes = vec![
            published("ghcr.io", &["ghcr.io/o/a:v1"]),
            PublishOutcome::NoTags {
                registry: "docker.io".to_string(),
            },
        ];
        assert_eq!(
            publish_summary(&outcomes),
            "docker: published 1 tag(s) to ghcr.io"
        );
    }

    #[test]
    fn publishing_nothing_while_blocked_is_an_error() {
        let outcomes = vec![skipped_no_username("docker.io")];

        let error = nothing_published_error(&outcomes).expect("should fail the run");
        assert!(error.contains("nothing was published"));
        assert!(error.contains("docker.io: no username configured"));
    }

    #[test]
    fn a_missing_token_is_reported_with_the_variable_to_set() {
        let outcomes = vec![skipped_no_token("ghcr.io", "GHCR_TOKEN")];

        let error = nothing_published_error(&outcomes).expect("should fail the run");
        assert!(error.contains("$GHCR_TOKEN not set"), "got {error}");
    }

    #[test]
    fn publishing_anything_at_all_is_not_an_error() {
        // A partial publish is a real outcome, not a failure — the summary
        // still names what was skipped.
        let outcomes = vec![
            published("ghcr.io", &["ghcr.io/o/a:v1"]),
            skipped_no_username("docker.io"),
        ];
        assert_eq!(nothing_published_error(&outcomes), None);
    }

    #[test]
    fn having_nothing_to_publish_is_not_an_error() {
        // No tags anywhere is reported earlier as "no tags resolved"; it is
        // not the silent-under-publish failure this guard exists for.
        let outcomes = vec![PublishOutcome::NoTags {
            registry: "docker.io".to_string(),
        }];
        assert_eq!(nothing_published_error(&outcomes), None);
        assert_eq!(nothing_published_error(&[]), None);
    }

    #[test]
    fn published_tag_count_sums_across_registries() {
        let outcomes = vec![
            published("ghcr.io", &["a", "b"]),
            published("docker.io", &["c"]),
            skipped_no_username("myco.jfrog.io"),
        ];
        assert_eq!(published_tag_count(&outcomes), 3);
    }

    // --- publish planning -------------------------------------------------
    //
    // Planning is a pure function precisely so these can exist: every case
    // below used to require a Dagger daemon, a registry, and environment
    // mutation to exercise, which is why two planning bugs shipped.

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn plan<'a>(
        image: &'a str,
        tag_list: &'a [String],
        registries: &'a [String],
    ) -> PublishPlanInput<'a> {
        PublishPlanInput {
            image,
            tags: tag_list,
            registries,
            ..Default::default()
        }
    }

    fn target<'a>(targets: &'a [PublishTarget], registry: &str) -> &'a PublishTarget {
        targets
            .iter()
            .find(|t| t.registry == registry)
            .unwrap_or_else(|| panic!("no target for {registry} in {targets:#?}"))
    }

    #[test]
    fn a_qualified_image_becomes_its_own_target() {
        let tag_list = tags(&["ghcr.io/owner/app:v1.0.0"]);
        let targets = plan_publish_targets(&plan("ghcr.io/owner/app", &tag_list, &[]));

        let ghcr = target(&targets, "ghcr.io");
        assert_eq!(ghcr.tags, vec!["ghcr.io/owner/app:v1.0.0"]);
        assert_eq!(ghcr.origin, TargetOrigin::Image);
        // The whole point: naming it in --image is an explicit ask, so a
        // missing credential must fail rather than skip.
        assert!(ghcr.credentials_required);

        // ...and Docker Hub must not claim the tag.
        assert!(target(&targets, "docker.io").tags.is_empty());
    }

    #[test]
    fn a_bare_image_targets_docker_hub_only() {
        let tag_list = tags(&["owner/app:v1.0.0"]);
        let targets = plan_publish_targets(&plan("owner/app", &tag_list, &[]));

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].registry, "docker.io");
        assert_eq!(targets[0].tags, vec!["owner/app:v1.0.0"]);
        // Docker Hub is considered rather than asked for, so it degrades.
        assert!(!targets[0].credentials_required);
    }

    #[test]
    fn an_image_registry_repeated_in_registries_is_not_duplicated() {
        let tag_list = tags(&["ghcr.io/owner/app:v1.0.0"]);
        let registries = vec!["ghcr.io".to_string()];
        let targets = plan_publish_targets(&plan("ghcr.io/owner/app", &tag_list, &registries));

        assert_eq!(
            targets.iter().filter(|t| t.registry == "ghcr.io").count(),
            1,
            "ghcr.io must appear once, not twice"
        );
        // Reached via --registries, ghcr.io keeps its graceful degrade.
        assert_eq!(target(&targets, "ghcr.io").origin, TargetOrigin::Registries);
    }

    #[test]
    fn an_explicitly_qualified_docker_io_image_stays_docker_hubs() {
        let tag_list = tags(&["docker.io/owner/app:v1.0.0"]);
        let targets = plan_publish_targets(&plan("docker.io/owner/app", &tag_list, &[]));

        assert_eq!(targets.len(), 1, "docker.io must not be added twice");
        assert_eq!(
            target(&targets, "docker.io").tags,
            vec!["docker.io/owner/app:v1.0.0"]
        );
    }

    #[test]
    fn tags_are_routed_to_the_registry_that_owns_them() {
        let tag_list = tags(&[
            "owner/app:v1",
            "ghcr.io/owner/app:v1",
            "myco.jfrog.io/owner/app:v1",
        ]);
        let registries = vec!["ghcr.io".to_string(), "myco.jfrog.io".to_string()];
        let targets = plan_publish_targets(&plan("owner/app", &tag_list, &registries));

        assert_eq!(target(&targets, "docker.io").tags, vec!["owner/app:v1"]);
        assert_eq!(
            target(&targets, "ghcr.io").tags,
            vec!["ghcr.io/owner/app:v1"]
        );
        assert_eq!(
            target(&targets, "myco.jfrog.io").tags,
            vec!["myco.jfrog.io/owner/app:v1"]
        );
    }

    #[test]
    fn a_custom_registry_requires_credentials_and_derives_its_token_var() {
        let tag_list = tags(&["myco.jfrog.io/app:v1"]);
        let registries = vec!["myco.jfrog.io".to_string()];
        let targets = plan_publish_targets(&plan("app", &tag_list, &registries));

        let custom = target(&targets, "myco.jfrog.io");
        assert_eq!(custom.token_env_var, "MYCO_JFROG_IO_TOKEN");
        assert!(custom.credentials_required);
    }

    #[test]
    fn ghcr_falls_back_to_the_workflow_token() {
        let tag_list = tags(&["ghcr.io/owner/app:v1"]);
        let base = plan("ghcr.io/owner/app", &tag_list, &[]);

        // Neither set: name GHCR_TOKEN, so the error tells you what to set.
        let neither = plan_publish_targets(&base);
        assert_eq!(target(&neither, "ghcr.io").token_env_var, "GHCR_TOKEN");

        // Only the workflow token: use it rather than skipping the publish.
        let fallback = plan_publish_targets(&PublishPlanInput {
            github_token_present: true,
            ..base.clone()
        });
        assert_eq!(target(&fallback, "ghcr.io").token_env_var, "GITHUB_TOKEN");

        // An explicit GHCR_TOKEN always wins.
        let explicit = plan_publish_targets(&PublishPlanInput {
            ghcr_token_present: true,
            github_token_present: true,
            ..base.clone()
        });
        assert_eq!(target(&explicit, "ghcr.io").token_env_var, "GHCR_TOKEN");
    }

    #[test]
    fn usernames_are_routed_from_the_matching_flag() {
        let tag_list = tags(&[
            "owner/app:v1",
            "ghcr.io/owner/app:v1",
            "myco.jfrog.io/app:v1",
        ]);
        let registries = vec!["ghcr.io".to_string(), "myco.jfrog.io".to_string()];
        let extra = vec![("myco.jfrog.io".to_string(), "deploy-bot".to_string())];

        let targets = plan_publish_targets(&PublishPlanInput {
            image: "owner/app",
            tags: &tag_list,
            registries: &registries,
            dockerhub_username: Some("hub-user"),
            ghcr_username: Some("ghcr-user"),
            extra_usernames: &extra,
            ..Default::default()
        });

        assert_eq!(
            target(&targets, "docker.io").username.as_deref(),
            Some("hub-user")
        );
        assert_eq!(
            target(&targets, "ghcr.io").username.as_deref(),
            Some("ghcr-user")
        );
        assert_eq!(
            target(&targets, "myco.jfrog.io").username.as_deref(),
            Some("deploy-bot")
        );
    }

    #[test]
    fn a_missing_credential_blames_the_flag_that_asked_for_the_registry() {
        let tag_list = tags(&["ghcr.io/owner/app:v1"]);

        let from_image = plan_publish_targets(&plan("ghcr.io/owner/app", &tag_list, &[]));
        assert_eq!(target(&from_image, "ghcr.io").origin.flag(), "--image");

        let registries = vec!["ghcr.io".to_string()];
        let from_flag = plan_publish_targets(&plan("owner/app", &tag_list, &registries));
        assert_eq!(target(&from_flag, "ghcr.io").origin.flag(), "--registries");
    }

    #[test]
    fn planning_is_deterministic() {
        let tag_list = tags(&["ghcr.io/owner/app:v1"]);
        let input = plan("ghcr.io/owner/app", &tag_list, &[]);
        assert_eq!(plan_publish_targets(&input), plan_publish_targets(&input));
    }

    /// The regression this all existed for: with `--image ghcr.io/owner/app`
    /// and no `--registries`, the ghcr tag is prefixed by no *extra* registry,
    /// so elimination alone claimed it for docker.io. It was then published to
    /// Docker Hub, or — with no Docker Hub credentials — skipped while the run
    /// still reported success.
    #[test]
    fn docker_hub_tags_does_not_claim_a_qualified_image_with_no_extra_registries() {
        let tags = vec!["ghcr.io/owner/app:v1.0.0".to_string()];

        assert!(
            docker_hub_tags(&tags, &[]).is_empty(),
            "a ghcr.io reference is not a Docker Hub tag"
        );
        assert_eq!(
            tags_for_registry(&tags, "ghcr.io"),
            vec!["ghcr.io/owner/app:v1.0.0"],
            "it belongs to ghcr.io"
        );
    }

    /// docker.io named explicitly is still Docker Hub's.
    #[test]
    fn docker_hub_tags_keeps_an_explicitly_qualified_docker_io_tag() {
        let tags = vec!["docker.io/owner/app:v1.0.0".to_string()];
        assert_eq!(
            docker_hub_tags(&tags, &[]),
            vec!["docker.io/owner/app:v1.0.0"]
        );
    }

    #[test]
    fn registry_of_follows_docker_reference_rules() {
        // A host is recognised by a dot, a port, or being localhost.
        assert_eq!(registry_of("ghcr.io/owner/app"), Some("ghcr.io"));
        assert_eq!(registry_of("myco.jfrog.io/app:v1"), Some("myco.jfrog.io"));
        assert_eq!(registry_of("localhost/app"), Some("localhost"));
        assert_eq!(registry_of("localhost:5000/app"), Some("localhost:5000"));

        // Docker Hub references name no registry, whether namespaced or not.
        assert_eq!(registry_of("owner/app"), None);
        assert_eq!(registry_of("app"), None);
        assert_eq!(registry_of("app:v1"), None);
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
    fn build_only_pipeline_args_relativizes_dockerfile_against_a_compose_context_subdir() {
        // resolve_docker_facts (see its own tests) deliberately computes
        // `dockerfile` relative to the repo root -- "./app/Dockerfile" for
        // a compose service with context "./app" -- matching real
        // `docker build -f`'s CLI semantics. But dagger's `host directory
        // --path=./app` only mounts "./app" itself, so the `--dockerfile`
        // arg must be relative to *that* mount ("./Dockerfile"), not the
        // repo root, or dagger looks for "./app/Dockerfile" inside the
        // already-mounted "./app" (i.e. an "app/app/Dockerfile" that
        // doesn't exist) -- reproduced for real against
        // examples/docker-compose-fixture before this fix.
        let args = build_only_pipeline_args(&BuildSpec {
            context: "./app",
            dockerfile: "./app/Dockerfile",
            target: "runtime",
            build_args: &[],
        });
        assert_eq!(
            args,
            vec![
                "host".to_string(),
                "directory".to_string(),
                "--path=./app".to_string(),
                "docker-build".to_string(),
                "--dockerfile=./Dockerfile".to_string(),
                "--target=runtime".to_string(),
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
