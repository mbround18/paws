//! Native Helm chart lint/package/publish support. Lint/package ported from
//! `mbround18/helm-charts`'s own `tools/chart_tasks.py`; publish ported from
//! that repo's `tools/release_charts.py` (both real usage — see
//! `docs/ROADMAP.md`'s Helm-chart-support gap), not from `gh-reusable`,
//! which has no Helm function to parity-port from. `publish_pipeline_args`
//! covers a standard Helm chart repository (per-chart GitHub Releases +
//! `index.yaml`, real enough that `helm repo add` against it works) —
//! deliberately not the HTML catalog page `release_charts.py` also
//! generates, which is that repo's own site feature, not a generic Helm
//! concern. That repo's separate Python test suite (which doesn't fit
//! `paws-python`'s fixed pipeline shape either) stays out of scope too.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The Helm builder Dockerfile, embedded at compile time from
/// `builders/helm/Dockerfile` — same reason as `paws-tauri`/`paws-flatpak`:
/// `paws` runs from inside whatever *target* repo it's checking, not from
/// inside `paws`'s own source tree, so a repo-relative `builders/helm` path
/// would resolve against the wrong directory.
const HELM_DOCKERFILE: &str = include_str!("../../../builders/helm/Dockerfile");

/// Writes the embedded Helm builder Dockerfile to a temp directory and
/// returns that directory's path, suitable for [`lint_pipeline_args`]/
/// [`package_pipeline_args`]'s `builder_dir` argument.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("paws-builders").join("helm");
    std::fs::create_dir_all(&dir)
        .context("failed to create temp dir for the helm builder Dockerfile")?;
    std::fs::write(dir.join("Dockerfile"), HELM_DOCKERFILE)
        .context("failed to write the helm builder Dockerfile")?;
    Ok(dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmChart {
    /// Directory basename, e.g. "mongo" for "charts/mongo" — also the join
    /// key [`HelmChart::local_dependencies`] entries are resolved against,
    /// since a `file://../mongo` dependency reference names a directory,
    /// not necessarily the `name:` a chart's own `Chart.yaml` declares.
    pub name: String,
    /// Path relative to the project root, e.g. "charts/mongo", or "." for a
    /// single-chart repo with `Chart.yaml` at the root.
    pub dir: String,
    pub has_dependencies: bool,
    /// Directory basenames of this chart's local (`file://../<dir>`)
    /// dependencies — used only to order `helm dependency build` calls, see
    /// [`topological_order`].
    local_dependencies: Vec<String>,
    /// `Chart.yaml`'s own `name:`/`version:` — usually identical to `name`
    /// (the directory basename) but not guaranteed to be, so [`tag`](Self::tag)/
    /// [`archive_file_name`](Self::archive_file_name) use these, not `name`.
    chart_name: String,
    chart_version: String,
}

impl HelmChart {
    /// The per-chart GitHub Release tag `paws helm --publish` creates —
    /// `<chart-name>-<chart-version>`, matching
    /// `mbround18/helm-charts`'s own `tools/release_charts.py` convention
    /// (`ChartPackage.tag_name`).
    pub fn tag(&self) -> String {
        format!("{}-{}", self.chart_name, self.chart_version)
    }

    /// The archive `helm package` produces for this chart — Helm's own
    /// naming convention (`<name>-<version>.tgz`), verified for real
    /// against `mbround18/helm-charts`'s actual packaged output this
    /// session.
    pub fn archive_file_name(&self) -> String {
        format!("{}-{}.tgz", self.chart_name, self.chart_version)
    }
}

pub struct HelmProject {
    /// Discovered charts, topologically ordered so a local dependency is
    /// always processed before whatever depends on it — matters for a
    /// chain more than one level deep (e.g. `bubbles-ttrpg` -> `mongo` ->
    /// `gitops-tools` in `mbround18/helm-charts` itself; alphabetical
    /// discovery order alone gets this wrong).
    pub charts: Vec<HelmChart>,
    /// Every distinct `http(s)://` `repository:` URL referenced across all
    /// discovered charts' `Chart.yaml`/`Chart.lock` (sorted, deduplicated).
    /// A fresh `helm` install has no repositories configured at all —
    /// `helm dependency build`/`update` fails outright on a remote
    /// dependency ("no repository definition ... please add the missing
    /// repos via 'helm repo add'") unless each one is added first, verified
    /// for real against `mbround18/helm-charts`'s own `meilisearch` chart
    /// (depends on `istio-ingress` via
    /// `https://mbround18.github.io/helm-charts`). Mirrors that repo's own
    /// `.github/actions/setup-helm` composite action, which does the exact
    /// same Chart.yaml/Chart.lock scan + `helm repo add --force-update` for
    /// this same reason.
    pub remote_repositories: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawChartYaml {
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
}

#[derive(Deserialize)]
struct RawDependency {
    #[serde(default)]
    repository: String,
}

fn discover_chart_relative_dirs(dir: &Path) -> Vec<String> {
    let charts_root = dir.join("charts");
    let Ok(entries) = std::fs::read_dir(&charts_root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.path().join("Chart.yaml").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names.into_iter().map(|n| format!("charts/{n}")).collect()
}

/// A Helm-chart repo: either a single chart at the root (`Chart.yaml`) or a
/// monorepo of charts under `charts/*/Chart.yaml` (`mbround18/helm-charts`'s
/// own layout, and Helm chart-releaser's conventional one).
pub fn is_helm_project(dir: &Path) -> bool {
    dir.join("Chart.yaml").is_file() || !discover_chart_relative_dirs(dir).is_empty()
}

/// Parses a `Chart.yaml`/`Chart.lock` at `path` (both share the same
/// `dependencies: [{repository, ...}]` shape; unrecognized fields, like
/// `Chart.lock`'s `digest`/`generated`, are ignored) and returns it, or a
/// default (empty) value if `path` doesn't exist.
fn read_chart_yaml(path: &Path) -> Result<RawChartYaml> {
    if !path.is_file() {
        return Ok(RawChartYaml::default());
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_yaml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
}

/// Loads the chart at `root`/`rel_dir`, returning it alongside every
/// distinct `http(s)://` `repository:` URL its `Chart.yaml`/`Chart.lock`
/// reference — see [`HelmProject::remote_repositories`].
fn load_chart(root: &Path, rel_dir: &str) -> Result<(HelmChart, Vec<String>)> {
    let chart_dir = root.join(rel_dir);
    let chart_yaml_path = chart_dir.join("Chart.yaml");
    anyhow::ensure!(
        chart_yaml_path.is_file(),
        "no Chart.yaml found at {}",
        chart_yaml_path.display()
    );
    let chart_yaml = read_chart_yaml(&chart_yaml_path)?;
    let chart_name = chart_yaml
        .name
        .clone()
        .with_context(|| format!("{} missing name", chart_yaml_path.display()))?;
    let chart_version = chart_yaml
        .version
        .clone()
        .with_context(|| format!("{} missing version", chart_yaml_path.display()))?;

    let mut dependencies = chart_yaml.dependencies;
    let has_dependencies = !dependencies.is_empty();
    dependencies.extend(read_chart_yaml(&chart_dir.join("Chart.lock"))?.dependencies);

    let local_dependencies: Vec<String> = dependencies
        .iter()
        .filter_map(|d| d.repository.strip_prefix("file://"))
        .filter_map(|rel| Path::new(rel).file_name())
        .filter_map(|name| name.to_str())
        .map(|s| s.to_string())
        .collect();

    let remote_repositories: Vec<String> = dependencies
        .iter()
        .map(|d| d.repository.as_str())
        .filter(|repo| repo.starts_with("http://") || repo.starts_with("https://"))
        .map(|s| s.to_string())
        .collect();

    let name = rel_dir
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(rel_dir)
        .to_string();

    Ok((
        HelmChart {
            has_dependencies,
            name,
            dir: rel_dir.to_string(),
            local_dependencies,
            chart_name,
            chart_version,
        },
        remote_repositories,
    ))
}

/// Kahn's algorithm over `charts`' local `file://` dependency edges, keyed
/// by directory basename. A dependency that isn't itself one of the
/// discovered charts (a remote-repository dependency, or a local path
/// outside `charts/`) simply carries no ordering constraint. Any leftover
/// cycle (which `helm dependency build` would itself error on) still gets
/// appended rather than silently dropped, so every discovered chart is
/// guaranteed to appear exactly once in the result.
fn topological_order(charts: Vec<HelmChart>) -> Vec<HelmChart> {
    let index: HashMap<&str, usize> = charts
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i))
        .collect();

    let mut in_degree = vec![0usize; charts.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); charts.len()];
    for (i, chart) in charts.iter().enumerate() {
        for dep_name in &chart.local_dependencies {
            if let Some(&dep_idx) = index.get(dep_name.as_str())
                && dep_idx != i
            {
                dependents[dep_idx].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut queue: VecDeque<usize> = (0..charts.len()).filter(|&i| in_degree[i] == 0).collect();
    let mut visited = vec![false; charts.len()];
    let mut order = Vec::with_capacity(charts.len());

    while let Some(i) = queue.pop_front() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        order.push(i);
        for &dependent in &dependents[i] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }
    for (i, seen) in visited.iter().enumerate() {
        if !seen {
            order.push(i);
        }
    }

    let mut slots: Vec<Option<HelmChart>> = charts.into_iter().map(Some).collect();
    order
        .into_iter()
        .map(|i| slots[i].take().expect("each index appears once in `order`"))
        .collect()
}

/// Finds Helm chart(s) in `dir` and returns them in dependency-safe order —
/// see [`topological_order`].
pub fn detect_project(dir: &Path) -> Result<HelmProject> {
    let mut rel_dirs = discover_chart_relative_dirs(dir);
    if rel_dirs.is_empty() && dir.join("Chart.yaml").is_file() {
        rel_dirs.push(".".to_string());
    }
    if rel_dirs.is_empty() {
        anyhow::bail!(
            "no Helm chart(s) found in {} (checked charts/*/Chart.yaml and ./Chart.yaml)",
            dir.display()
        );
    }

    let loaded: Vec<(HelmChart, Vec<String>)> = rel_dirs
        .iter()
        .map(|rel_dir| load_chart(dir, rel_dir))
        .collect::<Result<_>>()?;

    let mut remote_repositories: Vec<String> = loaded
        .iter()
        .flat_map(|(_, repos)| repos.iter().cloned())
        .collect();
    remote_repositories.sort();
    remote_repositories.dedup();

    let charts: Vec<HelmChart> = loaded.into_iter().map(|(chart, _)| chart).collect();

    Ok(HelmProject {
        charts: topological_order(charts),
        remote_repositories,
    })
}

fn container_prefix(builder_dir: &str, source_dir: &str) -> Vec<String> {
    let created_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build_args =
        format!("BUILDER_VERSION=dev,BUILDER_REVISION=unknown,BUILDER_CREATED={created_unix}");

    vec![
        "host".into(),
        "directory".into(),
        format!("--path={builder_dir}"),
        "docker-build".into(),
        format!("--build-args={build_args}"),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
    ]
}

fn push_exec(args: &mut Vec<String>, command_args: &[&str]) {
    args.push("with-exec".into());
    args.push(format!("--args={}", command_args.join(",")));
}

/// Adds every one of `project.remote_repositories` via `helm repo add
/// --force-update repo-<i> <url>`, then a single `helm repo update` — a
/// fresh `helm` install (this builder's container, every single run) has no
/// repositories configured, so any chart with a remote (non-`file://`)
/// dependency fails `helm dependency build`/`lint`/`package` otherwise. See
/// [`HelmProject::remote_repositories`]'s doc comment. `--force-update`
/// mirrors `mbround18/helm-charts`'s own `setup-helm` composite action,
/// which does the same add-per-URL + update, so re-running this against an
/// already-configured repo (e.g. local iteration reusing a Dagger session)
/// doesn't fail on "repository already exists".
fn push_remote_repo_setup(args: &mut Vec<String>, project: &HelmProject) {
    for (i, url) in project.remote_repositories.iter().enumerate() {
        let repo_name = format!("repo-{i}");
        push_exec(
            args,
            &[
                "helm",
                "repo",
                "add",
                "--force-update",
                &repo_name,
                url.as_str(),
            ],
        );
    }
    if !project.remote_repositories.is_empty() {
        push_exec(args, &["helm", "repo", "update"]);
    }
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that builds the Helm builder from `builder_dir` (see
/// [`write_builder_dockerfile`]) and runs `helm lint` against every chart in
/// `project`, dependency-order — `helm dependency build --skip-refresh
/// <chart-dir>` first for any chart that declares dependencies (local or
/// remote), since `helm lint` on a chart with unresolved subchart references
/// fails.
pub fn lint_pipeline_args(
    project: &HelmProject,
    source_dir: &str,
    builder_dir: &str,
) -> Vec<String> {
    let mut args = container_prefix(builder_dir, source_dir);
    push_remote_repo_setup(&mut args, project);
    for chart in &project.charts {
        if chart.has_dependencies {
            push_exec(
                &mut args,
                &["helm", "dependency", "build", "--skip-refresh", &chart.dir],
            );
        }
        push_exec(&mut args, &["helm", "lint", &chart.dir]);
    }
    args.push("stdout".into());
    args
}

/// Same shape as [`lint_pipeline_args`], but also runs `helm package
/// <chart-dir> -d <container_output_dir>` for every chart after linting it
/// (packaging a chart that fails lint isn't useful), then exports
/// `container_output_dir` to `host_output_dir` — mirrors
/// `paws-release::build_binary`'s `file`/`export` tail, just over a whole
/// directory of `.tgz` packages instead of one binary.
pub fn package_pipeline_args(
    project: &HelmProject,
    source_dir: &str,
    builder_dir: &str,
    container_output_dir: &str,
    host_output_dir: &str,
) -> Vec<String> {
    let mut args = container_prefix(builder_dir, source_dir);
    push_remote_repo_setup(&mut args, project);
    push_exec(&mut args, &["mkdir", "-p", container_output_dir]);
    for chart in &project.charts {
        if chart.has_dependencies {
            push_exec(
                &mut args,
                &["helm", "dependency", "build", "--skip-refresh", &chart.dir],
            );
        }
        push_exec(&mut args, &["helm", "lint", &chart.dir]);
        push_exec(
            &mut args,
            &["helm", "package", &chart.dir, "-d", container_output_dir],
        );
    }
    args.extend([
        "directory".into(),
        format!("--path={container_output_dir}"),
        "export".into(),
        format!("--path={host_output_dir}"),
    ]);
    args
}

/// Where `paws helm --publish` is publishing to, and what it already knows
/// about the running index — grouped into one struct so
/// [`publish_pipeline_prefix`]/[`publish_packages_pipeline_args`]/
/// [`publish_index_pipeline_args`] don't each carry five near-identical
/// parameters individually.
#[derive(Clone, Copy)]
pub struct PublishTarget<'a> {
    /// GitHub owner/org the per-chart releases and `index.yaml` publish to.
    pub owner: &'a str,
    pub repo: &'a str,
    /// The real, currently-published `index.yaml` (fetched by the caller —
    /// see `crates/paws-cli`'s `Commands::Helm` handler), if one exists
    /// yet. Seeds the running index so already-published chart versions
    /// keep their real download URLs — see [`publish_pipeline_prefix`].
    pub existing_index_path: Option<&'a Path>,
    /// Where each chart's package subdirectory lives inside the container.
    pub container_packages_dir: &'a str,
    /// Where the running `index.yaml` lives inside the container.
    pub container_index_path: &'a str,
}

/// The shared prefix [`publish_packages_pipeline_args`]/
/// [`publish_index_pipeline_args`] both build on: builds the Helm builder,
/// then for every chart (dependency-order, same as
/// [`lint_pipeline_args`]/[`package_pipeline_args`]) packages it into its
/// *own* subdirectory under `container_packages_dir` — not the shared flat
/// directory [`package_pipeline_args`] uses — because each chart's Helm
/// repo index entry needs its own `--url` (its own GitHub Release download
/// URL), and `helm repo index <dir> --url <u>` assigns `<u>` to every chart
/// it finds in `<dir>`. One subdirectory per chart is what lets `helm repo
/// index` do this correctly without paws having to reimplement Helm's own
/// index-merging logic (`mbround18/helm-charts`'s own
/// `tools/release_charts.py` had to, for exactly this reason — it never
/// considered per-chart directories, so it hand-rolled index-merging in
/// pure Python instead of shelling out to real Helm tooling).
///
/// `helm repo index --merge <container_index_path>` runs once per chart,
/// threading the same running index file through each call — `--merge`
/// preserves every existing entry in the given file untouched, so charts
/// already indexed by an earlier chart's call (or seeded in from
/// `existing_index_path`, the real currently-published `index.yaml`) keep
/// their real, already-correct URLs. `--merge` is omitted only for the
/// very first chart when nothing was seeded (a brand-new chart repo with
/// no prior publish at all).
fn publish_pipeline_prefix(
    project: &HelmProject,
    source_dir: &str,
    builder_dir: &str,
    target: &PublishTarget,
) -> Vec<String> {
    let PublishTarget {
        owner,
        repo,
        existing_index_path,
        container_packages_dir,
        container_index_path,
    } = *target;

    let mut args = container_prefix(builder_dir, source_dir);
    push_remote_repo_setup(&mut args, project);
    push_exec(&mut args, &["mkdir", "-p", container_packages_dir]);

    // The parent directory of `container_index_path` needs to exist before
    // the first chart's `cp .../index.yaml <container_index_path>` runs —
    // caught for real against a live throwaway repo: with no existing index
    // seeded (a brand-new chart repo's first-ever publish), nothing else in
    // this chain ever creates it, and `cp` failed with "No such file or
    // directory".
    let container_index_dir = Path::new(container_index_path)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|p| !p.is_empty());
    if let Some(container_index_dir) = container_index_dir {
        push_exec(&mut args, &["mkdir", "-p", container_index_dir]);
    }

    // The seed (if any) is mounted at a *separate* path from
    // `container_index_path`, never written back onto directly — also
    // caught for real: `with-mounted-file` bind-mounts it, and `cp`
    // overwriting a bind-mounted path fails with "File exists" (unlike a
    // plain file `cp` created itself, which overwrites fine). The very
    // first chart merges from the seed path if one exists; every chart
    // after that (and the seed-less first chart) merges from
    // `container_index_path`, an ordinary file by then.
    let seed_path = existing_index_path.map(|existing_index_path| {
        let seed_path = format!("{container_index_path}.seed");
        args.push("with-mounted-file".into());
        args.push(format!("--path={seed_path}"));
        args.push(format!("--source={}", existing_index_path.display()));
        seed_path
    });
    let mut merge_from: Option<String> = seed_path;

    for chart in &project.charts {
        let chart_out_dir = format!("{container_packages_dir}/{}", chart.name);
        if chart.has_dependencies {
            push_exec(
                &mut args,
                &["helm", "dependency", "build", "--skip-refresh", &chart.dir],
            );
        }
        push_exec(&mut args, &["helm", "lint", &chart.dir]);
        push_exec(&mut args, &["mkdir", "-p", &chart_out_dir]);
        push_exec(
            &mut args,
            &["helm", "package", &chart.dir, "-d", &chart_out_dir],
        );

        let download_url = format!(
            "https://github.com/{owner}/{repo}/releases/download/{}",
            chart.tag()
        );
        if let Some(merge_from) = &merge_from {
            push_exec(
                &mut args,
                &[
                    "helm",
                    "repo",
                    "index",
                    &chart_out_dir,
                    "--url",
                    &download_url,
                    "--merge",
                    merge_from,
                ],
            );
        } else {
            push_exec(
                &mut args,
                &[
                    "helm",
                    "repo",
                    "index",
                    &chart_out_dir,
                    "--url",
                    &download_url,
                ],
            );
        }
        push_exec(
            &mut args,
            &[
                "cp",
                &format!("{chart_out_dir}/index.yaml"),
                container_index_path,
            ],
        );
        merge_from = Some(container_index_path.to_string());
    }

    args
}

/// Builds the `dagger core <chain>` argument list that runs
/// [`publish_pipeline_prefix`] and exports every chart's packaged
/// subdirectory (under `container_packages_dir`) to `host_packages_dir`.
/// Paired with [`publish_index_pipeline_args`] — `dagger core` chains are
/// strictly linear (each call consumes the previous one's result), so
/// exporting two independent things (the packages *and* the index file)
/// needs two separate chains; both share this identical prefix, so
/// Dagger's own engine-level content-addressed caching means the second
/// invocation's already-executed steps replay from cache rather than
/// re-running (the same caching `paws-tauri`'s builder doc comment already
/// relies on for its own single-chain case).
pub fn publish_packages_pipeline_args(
    project: &HelmProject,
    source_dir: &str,
    builder_dir: &str,
    target: &PublishTarget,
    host_packages_dir: &str,
) -> Vec<String> {
    let mut args = publish_pipeline_prefix(project, source_dir, builder_dir, target);
    args.extend([
        "directory".into(),
        format!("--path={}", target.container_packages_dir),
        "export".into(),
        format!("--path={host_packages_dir}"),
    ]);
    args
}

/// Same as [`publish_packages_pipeline_args`], but exports the final merged
/// `index.yaml` (a single file, so `file`/`export` — mirrors
/// `paws-release::build_binary`'s tail for the same reason) to
/// `host_index_path` instead.
pub fn publish_index_pipeline_args(
    project: &HelmProject,
    source_dir: &str,
    builder_dir: &str,
    target: &PublishTarget,
    host_index_path: &str,
) -> Vec<String> {
    let mut args = publish_pipeline_prefix(project, source_dir, builder_dir, target);
    args.extend([
        "file".into(),
        format!("--path={}", target.container_index_path),
        "export".into(),
        format!("--path={host_index_path}"),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-helm-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_chart(root: &Path, rel_dir: &str, chart_yaml: &str) {
        let dir = root.join(rel_dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Chart.yaml"), chart_yaml).unwrap();
    }

    #[test]
    fn helm_builder_dockerfile_exists() {
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("builders/helm")
            .join("Dockerfile");
        assert!(dockerfile.is_file(), "missing {dockerfile:?}");
    }

    #[test]
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, HELM_DOCKERFILE);
    }

    #[test]
    fn detects_no_helm_project_without_charts() {
        let dir = temp_dir("no-project");
        assert!(!is_helm_project(&dir));
        assert!(detect_project(&dir).is_err());
    }

    #[test]
    fn detects_single_chart_at_root() {
        let dir = temp_dir("root-chart");
        write_chart(&dir, ".", "apiVersion: v2\nname: solo\nversion: 0.1.0\n");
        assert!(is_helm_project(&dir));
        let project = detect_project(&dir).unwrap();
        assert_eq!(project.charts.len(), 1);
        assert_eq!(project.charts[0].dir, ".");
        assert!(!project.charts[0].has_dependencies);
    }

    #[test]
    fn discovers_monorepo_charts_directory() {
        let dir = temp_dir("monorepo");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        write_chart(
            &dir,
            "charts/b",
            "apiVersion: v2\nname: b\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        assert_eq!(project.charts.len(), 2);
        assert_eq!(project.charts[0].name, "a");
        assert_eq!(project.charts[1].name, "b");
    }

    #[test]
    fn orders_local_file_dependencies_before_dependents() {
        let dir = temp_dir("topo");
        // Discovery order is alphabetical (bubbles, gitops, mongo), but the
        // real dependency chain is bubbles -> mongo -> gitops - the output
        // order must respect that, not alphabetical discovery order.
        write_chart(
            &dir,
            "charts/bubbles",
            "apiVersion: v2\nname: bubbles\nversion: 0.1.0\ndependencies:\n- name: mongo\n  version: 0.1.0\n  repository: file://../mongo\n",
        );
        write_chart(
            &dir,
            "charts/gitops",
            "apiVersion: v2\nname: gitops\nversion: 0.1.0\n",
        );
        write_chart(
            &dir,
            "charts/mongo",
            "apiVersion: v2\nname: mongo\nversion: 0.1.0\ndependencies:\n- name: gitops\n  version: 0.1.0\n  repository: file://../gitops\n",
        );

        let project = detect_project(&dir).unwrap();
        let order: Vec<&str> = project.charts.iter().map(|c| c.name.as_str()).collect();
        let gitops_pos = order.iter().position(|&n| n == "gitops").unwrap();
        let mongo_pos = order.iter().position(|&n| n == "mongo").unwrap();
        let bubbles_pos = order.iter().position(|&n| n == "bubbles").unwrap();
        assert!(
            gitops_pos < mongo_pos,
            "gitops must build before mongo: {order:?}"
        );
        assert!(
            mongo_pos < bubbles_pos,
            "mongo must build before bubbles: {order:?}"
        );
    }

    #[test]
    fn remote_dependencies_are_not_treated_as_ordering_constraints() {
        let dir = temp_dir("remote-dep");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\ndependencies:\n- name: istio-ingress\n  version: 0.1.0\n  repository: https://example.com/charts\n",
        );
        let project = detect_project(&dir).unwrap();
        assert_eq!(project.charts.len(), 1);
        assert!(project.charts[0].has_dependencies);
        assert_eq!(
            project.remote_repositories,
            vec!["https://example.com/charts".to_string()]
        );
    }

    #[test]
    fn remote_repositories_are_deduplicated_and_sorted() {
        let dir = temp_dir("remote-dedup");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\ndependencies:\n- name: x\n  version: 0.1.0\n  repository: https://b.example.com\n",
        );
        write_chart(
            &dir,
            "charts/b",
            "apiVersion: v2\nname: b\nversion: 0.1.0\ndependencies:\n- name: y\n  version: 0.1.0\n  repository: https://a.example.com\n- name: z\n  version: 0.1.0\n  repository: https://b.example.com\n",
        );
        let project = detect_project(&dir).unwrap();
        assert_eq!(
            project.remote_repositories,
            vec![
                "https://a.example.com".to_string(),
                "https://b.example.com".to_string()
            ]
        );
    }

    #[test]
    fn chart_lock_dependencies_also_contribute_remote_repositories() {
        let dir = temp_dir("chart-lock");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        fs::write(
            dir.join("charts/a/Chart.lock"),
            "dependencies:\n- name: x\n  repository: https://locked.example.com\n  version: 0.1.0\ndigest: sha256:deadbeef\ngenerated: \"2026-01-01T00:00:00Z\"\n",
        )
        .unwrap();
        let project = detect_project(&dir).unwrap();
        assert_eq!(
            project.remote_repositories,
            vec!["https://locked.example.com".to_string()]
        );
    }

    #[test]
    fn lint_pipeline_adds_remote_repos_before_any_chart_work() {
        let dir = temp_dir("lint-remote-repo");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\ndependencies:\n- name: x\n  version: 0.1.0\n  repository: https://example.com/charts\n",
        );
        let project = detect_project(&dir).unwrap();
        let args = lint_pipeline_args(&project, "/host/src", "/tmp/some-builder-dir");

        assert!(args.contains(
            &"--args=helm,repo,add,--force-update,repo-0,https://example.com/charts".to_string()
        ));
        assert!(args.contains(&"--args=helm,repo,update".to_string()));
        let repo_add_pos = args
            .iter()
            .position(|a| a.starts_with("--args=helm,repo,add"))
            .unwrap();
        let dep_build_pos = args
            .iter()
            .position(|a| a.starts_with("--args=helm,dependency,build"))
            .unwrap();
        assert!(
            repo_add_pos < dep_build_pos,
            "repo add must run before any chart's dependency build"
        );
    }

    #[test]
    fn lint_pipeline_skips_repo_setup_when_no_remote_dependencies() {
        let dir = temp_dir("lint-no-remote-repo");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let args = lint_pipeline_args(&project, "/host/src", "/tmp/some-builder-dir");
        assert!(!args.iter().any(|a| a.starts_with("--args=helm,repo")));
    }

    #[test]
    fn lint_pipeline_runs_dependency_build_only_when_needed() {
        let dir = temp_dir("lint-args");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\ndependencies:\n- name: b\n  version: 0.1.0\n  repository: file://../b\n",
        );
        write_chart(
            &dir,
            "charts/b",
            "apiVersion: v2\nname: b\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let args = lint_pipeline_args(&project, "/host/src", "/tmp/some-builder-dir");

        assert_eq!(args[2], "--path=/tmp/some-builder-dir");
        assert_eq!(args[3], "docker-build");
        // b (no deps) builds first per topological order, then a (which
        // does need `helm dependency build` before `helm lint`).
        assert!(args.contains(&"--args=helm,lint,charts/b".to_string()));
        assert!(args.contains(&"--args=helm,dependency,build,--skip-refresh,charts/a".to_string()));
        assert!(args.contains(&"--args=helm,lint,charts/a".to_string()));
        assert!(
            !args.contains(&"--args=helm,dependency,build,--skip-refresh,charts/b".to_string())
        );
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn package_pipeline_exports_the_output_directory() {
        let dir = temp_dir("package-args");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let args = package_pipeline_args(
            &project,
            "/host/src",
            "/tmp/some-builder-dir",
            "/out",
            "/host/tmp",
        );

        assert!(args.contains(&"--args=helm,lint,charts/a".to_string()));
        assert!(args.contains(&"--args=helm,package,charts/a,-d,/out".to_string()));
        let lint_pos = args
            .iter()
            .position(|a| a == "--args=helm,lint,charts/a")
            .unwrap();
        let package_pos = args
            .iter()
            .position(|a| a == "--args=helm,package,charts/a,-d,/out")
            .unwrap();
        assert!(lint_pos < package_pos, "lint must run before package");
        assert_eq!(args[args.len() - 4], "directory");
        assert_eq!(args[args.len() - 3], "--path=/out");
        assert_eq!(args[args.len() - 2], "export");
        assert_eq!(args.last(), Some(&"--path=/host/tmp".to_string()));
    }

    #[test]
    fn chart_tag_and_archive_file_name_use_chart_yaml_metadata() {
        let dir = temp_dir("tag-and-archive");
        write_chart(
            &dir,
            "charts/mongo",
            "apiVersion: v2\nname: mongo\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let chart = &project.charts[0];
        assert_eq!(chart.tag(), "mongo-0.1.0");
        assert_eq!(chart.archive_file_name(), "mongo-0.1.0.tgz");
    }

    #[test]
    fn publish_pipeline_packages_each_chart_into_its_own_subdirectory() {
        let dir = temp_dir("publish-args");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        write_chart(
            &dir,
            "charts/b",
            "apiVersion: v2\nname: b\nversion: 0.2.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let target = PublishTarget {
            owner: "mbround18",
            repo: "helm-charts",
            existing_index_path: None,
            container_packages_dir: "/out",
            container_index_path: "/idx/index.yaml",
        };
        let args = publish_packages_pipeline_args(
            &project,
            "/host/src",
            "/tmp/some-builder-dir",
            &target,
            "/host/packages",
        );

        assert!(args.contains(&"--args=helm,package,charts/a,-d,/out/a".to_string()));
        assert!(args.contains(&"--args=helm,package,charts/b,-d,/out/b".to_string()));
        // First chart processed gets no --merge (nothing seeded yet); the
        // second does, since the first chart's index.yaml was copied into
        // the running index file by then.
        assert!(args.contains(
            &"--args=helm,repo,index,/out/a,--url,https://github.com/mbround18/helm-charts/releases/download/a-0.1.0"
                .to_string()
        ));
        assert!(args.contains(
            &"--args=helm,repo,index,/out/b,--url,https://github.com/mbround18/helm-charts/releases/download/b-0.2.0,--merge,/idx/index.yaml"
                .to_string()
        ));
        assert_eq!(args[args.len() - 4], "directory");
        assert_eq!(args[args.len() - 3], "--path=/out");
        assert_eq!(args.last(), Some(&"--path=/host/packages".to_string()));
    }

    #[test]
    fn publish_index_pipeline_seeds_from_an_existing_index_and_exports_the_file() {
        let dir = temp_dir("publish-index-args");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let existing = dir.join("seed-index.yaml");
        fs::write(&existing, "apiVersion: v1\nentries: {}\n").unwrap();

        let target = PublishTarget {
            owner: "mbround18",
            repo: "helm-charts",
            existing_index_path: Some(&existing),
            container_packages_dir: "/out",
            container_index_path: "/idx/index.yaml",
        };
        let args = publish_index_pipeline_args(
            &project,
            "/host/src",
            "/tmp/some-builder-dir",
            &target,
            "/host/index.yaml",
        );

        // The seed mounts at a *separate* path from the running index file
        // - never written back onto directly, since a bind-mounted file
        // can't be `cp`-overwritten (caught for real against a live
        // throwaway repo: "cp: can't create '/idx/index.yaml': File exists").
        assert!(args.contains(&"with-mounted-file".to_string()));
        assert!(args.contains(&"--path=/idx/index.yaml.seed".to_string()));
        assert!(args.contains(&format!("--source={}", existing.display())));
        // Seeded from the start, so even the first (only) chart gets
        // --merge, from the seed path specifically.
        assert!(args.contains(
            &"--args=helm,repo,index,/out/a,--url,https://github.com/mbround18/helm-charts/releases/download/a-0.1.0,--merge,/idx/index.yaml.seed"
                .to_string()
        ));
        // The running index itself is a fresh `cp` target, not the mount.
        assert!(args.contains(&"--args=cp,/out/a/index.yaml,/idx/index.yaml".to_string()));
        assert_eq!(args[args.len() - 4], "file");
        assert_eq!(args[args.len() - 3], "--path=/idx/index.yaml");
        assert_eq!(args[args.len() - 2], "export");
        assert_eq!(args.last(), Some(&"--path=/host/index.yaml".to_string()));
    }

    #[test]
    fn publish_pipeline_merges_from_the_running_index_after_the_first_chart() {
        let dir = temp_dir("publish-args-multi-seeded");
        write_chart(
            &dir,
            "charts/a",
            "apiVersion: v2\nname: a\nversion: 0.1.0\n",
        );
        write_chart(
            &dir,
            "charts/b",
            "apiVersion: v2\nname: b\nversion: 0.2.0\n",
        );
        let project = detect_project(&dir).unwrap();
        let existing = dir.join("seed-index.yaml");
        fs::write(&existing, "apiVersion: v1\nentries: {}\n").unwrap();
        let target = PublishTarget {
            owner: "mbround18",
            repo: "helm-charts",
            existing_index_path: Some(&existing),
            container_packages_dir: "/out",
            container_index_path: "/idx/index.yaml",
        };
        let args = publish_packages_pipeline_args(
            &project,
            "/host/src",
            "/tmp/some-builder-dir",
            &target,
            "/host/packages",
        );

        // First chart merges from the seed; the second merges from the
        // running index the first chart's `cp` already wrote - not the seed
        // again, and not without --merge either.
        assert!(args.contains(
            &"--args=helm,repo,index,/out/a,--url,https://github.com/mbround18/helm-charts/releases/download/a-0.1.0,--merge,/idx/index.yaml.seed"
                .to_string()
        ));
        assert!(args.contains(
            &"--args=helm,repo,index,/out/b,--url,https://github.com/mbround18/helm-charts/releases/download/b-0.2.0,--merge,/idx/index.yaml"
                .to_string()
        ));
    }
}
