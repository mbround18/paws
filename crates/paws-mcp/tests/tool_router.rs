//! End-to-end MCP protocol tests for `PawsMcpServer`: drives it over a real
//! in-process transport (`tokio::io::duplex`, the same pattern `rmcp`'s own
//! test suite uses) with a real `ServiceExt::serve` client on the other end,
//! rather than calling generated tool methods directly — this exercises the
//! actual `tools/list`/`tools/call` wire protocol, not just the Rust
//! function underneath it.

use rmcp::{ServiceExt, model::CallToolRequestParams};

async fn connect() -> (
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_transport, client_transport) = tokio::io::duplex(4096);
    let server_task = tokio::spawn(async move {
        let server = paws_mcp::PawsMcpServer::new()
            .serve(server_transport)
            .await?;
        server.waiting().await?;
        anyhow::Ok(())
    });
    let client = ().serve(client_transport).await.expect("client connects");
    (client, server_task)
}

#[tokio::test]
async fn tools_list_exposes_every_paws_subcommand() {
    let (client, _server_task) = connect().await;

    let tools = client
        .list_tools(None)
        .await
        .expect("tools/list succeeds")
        .tools;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    for expected in [
        "ci",
        "docker",
        "semver",
        "init",
        "audit",
        "docs",
        "provision",
        "helm",
        "release",
        "workflow",
        "actions",
    ] {
        assert!(
            names.contains(&expected),
            "expected tools/list to include {expected:?}, got {names:?}"
        );
    }

    let _ = client.cancel().await;
}

#[tokio::test]
async fn calling_actions_returns_paws_up_metadata() {
    let (client, _server_task) = connect().await;

    let result = client
        .call_tool(CallToolRequestParams::new("actions"))
        .await
        .expect("actions is a pure metadata lookup, it should never fail");

    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or_default();
    assert!(text.contains("\"paws-up\""), "expected paws-up in: {text}");
    assert!(
        text.contains("\"version\""),
        "expected its version input/output in: {text}"
    );

    let _ = client.cancel().await;
}

#[tokio::test]
async fn calling_provision_with_no_toolchains_returns_an_mcp_error_not_a_crash() {
    let (client, _server_task) = connect().await;

    // provision bails immediately on empty --toolchains, before touching the
    // network or the filesystem — deterministic and fast, so it's a safe
    // real call to make over the wire rather than a fake/mocked one.
    //
    // `PawsMcpServer`'s tools return `Result<String, McpError>`; per rmcp's
    // `IntoCallToolResult for ErrorData` impl, an `Err` here becomes a real
    // JSON-RPC-level error response (`call_tool` itself returns `Err`), not
    // a "soft" `CallToolResult { is_error: true }` — this pins that actual
    // behavior rather than the `is_error` flag a different MCP framework
    // convention might use.
    let err = client
        .call_tool(CallToolRequestParams::new("provision"))
        .await
        .expect_err("run_provision's bail! should surface as a tool call error");

    let message = format!("{err}");
    assert!(
        message.contains("--toolchains is required"),
        "expected run_provision's error message in the tool call error, got: {message}"
    );

    let _ = client.cancel().await;
}

#[tokio::test]
async fn calling_an_unknown_tool_fails_cleanly() {
    let (client, _server_task) = connect().await;

    let err = client
        .call_tool(CallToolRequestParams::new("not-a-real-tool"))
        .await
        .expect_err("an unknown tool name should be rejected");
    assert!(
        format!("{err}").to_lowercase().contains("not found")
            || format!("{err}").to_lowercase().contains("unknown")
    );

    let _ = client.cancel().await;
}
