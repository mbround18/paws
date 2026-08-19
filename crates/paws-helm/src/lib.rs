//! Native Helm chart lint/package support. Ported from
//! `mbround18/helm-charts`'s own `tools/chart_tasks.py` (this repo's real
//! usage — see `docs/ROADMAP.md`'s Helm-chart-support gap), not from
//! `gh-reusable`, which has no Helm function to parity-port from.
//! Deliberately scoped to `helm lint`/`helm package` only, matching that
//! repo's Makefile's `lint-helm`/`build` targets — its separate Python test
//! suite (which doesn't fit `paws-python`'s fixed pipeline shape either) and
//! its `chart-releaser`/`gh-pages` publish flow (a GitHub-App-token based
//! mechanism, unrelated to registry auth) stay out of scope for this first
//! cut.

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
}

pub struct HelmProject {
    /// Discovered charts, topologically ordered so a local dependency is
    /// always processed before whatever depends on it — matters for a
    /// chain more than one level deep (e.g. `bubbles-ttrpg` -> `mongo` ->
    /// `gitops-tools` in `mbround18/helm-charts` itself; alphabetical
    /// discovery order alone gets this wrong).
    pub charts: Vec<HelmChart>,
}

#[derive(Deserialize, Default)]
struct RawChartYaml {
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

fn load_chart(root: &Path, rel_dir: &str) -> Result<HelmChart> {
    let chart_yaml_path = root.join(rel_dir).join("Chart.yaml");
    let contents = std::fs::read_to_string(&chart_yaml_path)
        .with_context(|| format!("failed to read {}", chart_yaml_path.display()))?;
    let raw: RawChartYaml = serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse {}", chart_yaml_path.display()))?;

    let local_dependencies: Vec<String> = raw
        .dependencies
        .iter()
        .filter_map(|d| d.repository.strip_prefix("file://"))
        .filter_map(|rel| Path::new(rel).file_name())
        .filter_map(|name| name.to_str())
        .map(|s| s.to_string())
        .collect();

    let name = rel_dir
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(rel_dir)
        .to_string();

    Ok(HelmChart {
        has_dependencies: !raw.dependencies.is_empty(),
        name,
        dir: rel_dir.to_string(),
        local_dependencies,
    })
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

    let charts: Vec<HelmChart> = rel_dirs
        .iter()
        .map(|rel_dir| load_chart(dir, rel_dir))
        .collect::<Result<_>>()?;

    Ok(HelmProject {
        charts: topological_order(charts),
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
}
