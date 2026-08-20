//! MCP server exposing every `paws` CLI subcommand as an MCP tool — calling
//! `paws-cli-core`'s `run_*` functions directly, never shelling out to the
//! `paws` binary itself.

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use paws_cli_core::{
    AuditArgs, CiArgs, DockerArgs, DocsArgs, HelmArgs, InitArgs, ProvisionArgs, ReleaseArgs,
    SemverArgs, WorkflowGenerateArgs,
};

/// Runs `f`, capturing anything it prints to stdout/stderr instead of
/// letting it reach the real process stdout — under stdio transport, process
/// stdout *is* the JSON-RPC channel, so a stray `println!` from a `run_*`
/// function would corrupt framing.
///
/// Limitation: `gag::BufferRedirect` redirects at the OS file-descriptor
/// level, which is process-wide. If two tool calls ever ran concurrently in
/// this process they would race over the same fd 1/2 redirection; this is an
/// accepted limitation for a single stdio MCP connection, which handles one
/// `tools/call` at a time in practice, not a concurrency guarantee.
async fn capture_output<F, Fut>(f: F) -> (anyhow::Result<()>, String)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    use std::io::Read;

    let stdout_gag = gag::BufferRedirect::stdout().ok();
    let stderr_gag = gag::BufferRedirect::stderr().ok();

    let result = f().await;

    let mut captured = String::new();
    if let Some(mut gag) = stdout_gag {
        let _ = gag.read_to_string(&mut captured);
    }
    if let Some(mut gag) = stderr_gag {
        let mut stderr_captured = String::new();
        let _ = gag.read_to_string(&mut stderr_captured);
        if !stderr_captured.is_empty() {
            captured.push_str(&stderr_captured);
        }
    }

    (result, captured)
}

fn tool_result(outcome: anyhow::Result<()>, captured: String) -> Result<String, McpError> {
    match outcome {
        Ok(()) => Ok(if captured.is_empty() {
            "ok (no output)".to_string()
        } else {
            captured
        }),
        Err(err) => Err(McpError::internal_error(
            format!("{err:#}\n\n{captured}"),
            None,
        )),
    }
}

#[derive(Debug, Clone)]
pub struct PawsMcpServer {
    tool_router: ToolRouter<Self>,
}

impl PawsMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for PawsMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl PawsMcpServer {
    #[tool(
        description = "Build and test a language target (node, rust, python, tauri, tauri-android, flatpak)."
    )]
    async fn ci(&self, Parameters(args): Parameters<CiArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_ci(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Build and gate a container image (build/tag/push across registries).")]
    async fn docker(&self, Parameters(args): Parameters<DockerArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_docker(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(
        description = "Compute the next semantic version from PR labels or an explicit increment."
    )]
    async fn semver(&self, Parameters(args): Parameters<SemverArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_semver(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Install the `dagger` CLI (most other tools need it on PATH).")]
    async fn init(&self, Parameters(args): Parameters<InitArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_init(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Run the audit/compliance scanner suite.")]
    async fn audit(&self, Parameters(args): Parameters<AuditArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_audit(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Publish generated docs (e.g. rustdoc) to GitHub Pages.")]
    async fn docs(&self, Parameters(args): Parameters<DocsArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_docs(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Provision toolchains concurrently (rust, node, python, ...).")]
    async fn provision(
        &self,
        Parameters(args): Parameters<ProvisionArgs>,
    ) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_provision(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(description = "Lint (and optionally package/publish) Helm chart(s).")]
    async fn helm(&self, Parameters(args): Parameters<HelmArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_helm(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(
        description = "Cross-target build, package, and publish a release binary to GitHub Releases."
    )]
    async fn release(&self, Parameters(args): Parameters<ReleaseArgs>) -> Result<String, McpError> {
        let (outcome, captured) = capture_output(|| paws_cli_core::run_release(args)).await;
        tool_result(outcome, captured)
    }

    #[tool(
        description = "Detect this repo's ecosystem(s) and generate a starter GitHub Actions workflow wiring in paws-up plus the matching paws subcommands."
    )]
    async fn workflow(
        &self,
        Parameters(args): Parameters<WorkflowGenerateArgs>,
    ) -> Result<String, McpError> {
        let (outcome, captured) =
            capture_output(|| paws_cli_core::run_workflow_generate(args)).await;
        tool_result(outcome, captured)
    }

    /// Pure metadata lookup (no subprocess/dagger output to capture), so
    /// this skips the `capture_output` wrapper every other tool here uses.
    #[tool(
        description = "List GitHub Actions this project ships (e.g. paws-up) with their inputs/outputs, for wiring into a consumer repo's CI."
    )]
    async fn actions(&self) -> Result<String, McpError> {
        let actions = paws_cli_core::action_metadata::discover_actions()
            .map_err(|e| McpError::internal_error(format!("{e:#}"), None))?;
        serde_json::to_string_pretty(&actions).map_err(|e| {
            McpError::internal_error(format!("failed to serialize action metadata: {e}"), None)
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PawsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "paws: run-anywhere CI/CD pipelines, backed by Dagger. Every tool here mirrors a \
             `paws` CLI subcommand exactly and calls the same Rust code directly — not a CLI \
             subprocess proxy.",
        )
    }
}

/// Runs the MCP server over stdio until the client disconnects.
pub async fn serve() -> anyhow::Result<()> {
    let server = PawsMcpServer::new();
    let running = server.serve(rmcp::transport::io::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialized: capture_output redirects process-wide fd 1/2, so two
    // copies racing in parallel test threads would corrupt each other's
    // buffers (the same limitation documented on capture_output itself).
    // An async-aware mutex, since the guard needs to stay held across the
    // `.await` inside capture_output.
    static CAPTURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Writes straight to the real fd 1, bypassing Rust's `io::stdout()`
    /// (which `cargo test`'s own output capture intercepts *before* it
    /// reaches the OS file descriptor — a plain `println!` here would never
    /// reach `gag`, which redirects at the fd level). This is also a more
    /// faithful stand-in for what `capture_output` actually has to handle in
    /// production: a subprocess (e.g. `dagger`) writing to its inherited
    /// stdout fd directly, not through Rust's stdout wrapper at all.
    fn write_directly_to_fd(fd_path: &str, text: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .open(fd_path)
            .expect("fd path is open")
            .write_all(text.as_bytes())
            .expect("write to fd succeeds");
    }

    #[tokio::test]
    async fn capture_output_collects_stdout_and_stderr_on_success() {
        let _guard = CAPTURE_LOCK.lock().await;
        let (outcome, captured) = capture_output(|| async {
            write_directly_to_fd("/dev/fd/1", "building thing\n");
            write_directly_to_fd("/dev/fd/2", "warning: thing is old\n");
            Ok(())
        })
        .await;

        assert!(outcome.is_ok());
        assert!(captured.contains("building thing"));
        assert!(captured.contains("warning: thing is old"));
    }

    #[tokio::test]
    async fn capture_output_still_captures_prints_before_a_failure() {
        let _guard = CAPTURE_LOCK.lock().await;
        let (outcome, captured) = capture_output(|| async {
            write_directly_to_fd("/dev/fd/1", "partial progress\n");
            anyhow::bail!("boom")
        })
        .await;

        assert!(outcome.is_err());
        assert!(captured.contains("partial progress"));
    }

    #[test]
    fn tool_result_maps_ok_with_output_to_the_captured_text() {
        let result = tool_result(Ok(()), "some output\n".to_string());
        assert_eq!(result.unwrap(), "some output\n");
    }

    #[test]
    fn tool_result_maps_ok_with_no_output_to_a_placeholder() {
        let result = tool_result(Ok(()), String::new());
        assert_eq!(result.unwrap(), "ok (no output)");
    }

    #[test]
    fn tool_result_maps_err_to_an_mcp_error_containing_the_message_and_captured_output() {
        let err = tool_result(
            Err(anyhow::anyhow!("--toolchains is required")),
            "some progress before it failed\n".to_string(),
        )
        .unwrap_err();

        let message = err.message.to_string();
        assert!(message.contains("--toolchains is required"));
        assert!(message.contains("some progress before it failed"));
    }
}
