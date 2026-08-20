//! Every other MCP test in this crate drives `PawsMcpServer` in-process over
//! a `tokio::io::duplex` pipe — none of them touch the actual `paws` binary,
//! which means `crates/paws-cli/src/main.rs`'s special-cased dispatch for
//! `Commands::Mcp(McpCommand::Serve(_))` (the whole reason `paws-mcp` avoids
//! a `paws-cli` dependency cycle — see that file's comment) has no coverage
//! anywhere else. This test spawns the real compiled binary with `mcp serve`
//! and drives it over real OS stdio pipes with raw JSON-RPC, mirroring the
//! framing `rmcp`'s own `tests/test_stdio_response_concurrency.rs` uses
//! (newline-delimited JSON, one message per line).

use std::{process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
};

const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// `CARGO_BIN_EXE_paws` isn't set here — Cargo only sets `CARGO_BIN_EXE_*`
/// for binaries owned by the package under test (or ones it directly
/// depends on as a build/dev artifact dependency), and `paws-mcp` depends on
/// `paws-cli-core`, not the `paws-cli` binary package. So this resolves the
/// path manually against whichever profile was actually built.
fn paws_binary_path() -> Option<std::path::PathBuf> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for profile in ["debug", "release"] {
        let candidate = manifest_dir.join("../../target").join(profile).join("paws");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

async fn send(stdin: &mut ChildStdin, value: serde_json::Value) {
    let mut line = serde_json::to_vec(&value).expect("serialize request");
    line.push(b'\n');
    stdin.write_all(&line).await.expect("write request");
}

async fn read_line<R>(reader: &mut BufReader<R>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = String::new();
    tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line))
        .await
        .expect("response arrives within the timeout")
        .expect("read the response line");
    line
}

#[tokio::test]
async fn real_paws_binary_serves_mcp_over_stdio() {
    let Some(binary) = paws_binary_path() else {
        // Mirrors this repo's existing convention (see ci.yaml's comment on
        // rustup/corepack/uv-dependent tests) of skipping gracefully rather
        // than failing when an external prerequisite isn't present — here,
        // a `paws-cli` build. `cargo test -p paws-mcp` alone doesn't build
        // the `paws` binary; running the workspace build first does.
        eprintln!(
            "skipping real_paws_binary_serves_mcp_over_stdio: no target/{{debug,release}}/paws \
             binary found — run `cargo build -p paws-cli` (or `cargo build --workspace`) first"
        );
        return;
    };

    let mut child = Command::new(&binary)
        .arg("mcp")
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the real paws binary");

    let mut stdin = child.stdin.take().expect("child stdin");
    let mut reader = BufReader::new(child.stdout.take().expect("child stdout"));

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "paws-mcp-subprocess-test", "version": "0.0.0" }
            }
        }),
    )
    .await;

    let init_response = read_line(&mut reader).await;
    assert!(
        init_response.contains("\"result\""),
        "unexpected initialize response from the real binary: {init_response}"
    );

    send(
        &mut stdin,
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;

    send(
        &mut stdin,
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;

    let list_response = read_line(&mut reader).await;
    assert!(
        list_response.contains("\"provision\""),
        "expected tools/list from the real binary to mention the provision tool, got: \
         {list_response}"
    );

    drop(stdin);
    let _ = child.start_kill();
    let _ = child.wait().await;
}
