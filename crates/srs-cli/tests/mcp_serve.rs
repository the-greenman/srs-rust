//! Phase 4 integration tests: `srs mcp serve` through the real binary.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use srs_repository::repository_lifecycle::{
    create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
    RepositoryMetadata,
};
use srs_repository::store::FileStore;

fn make_repo(dir: &std::path::Path) {
    create_repository_with_intent(
        &FileStore::new(dir),
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: uuid::Uuid::new_v4().to_string(),
                namespace: "com.example.mcpserve".into(),
                srs_version: "2.0-draft".into(),
                title: Some("MCP Serve Test".into()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                namespace: "com.example.mcpserve".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
}

/// Read newline-delimited JSON-RPC messages until one carries the wanted id.
fn read_until_id(reader: &mut impl BufRead, id: u64) -> serde_json::Value {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read from server");
        assert!(n > 0, "server closed stdout before answering id {id}");
        let value: serde_json::Value =
            serde_json::from_str(line.trim()).expect("stdout must carry only JSON-RPC");
        if value["id"] == serde_json::json!(id) {
            return value;
        }
    }
}

#[test]
fn mcp_serve_binary_initialize_handshake() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());

    let mut child = Command::new(env!("CARGO_BIN_EXE_srs"))
        .args(["mcp", "serve", "--repo"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"test-client","version":"0.0.0"}}}}}}"#
    )
    .unwrap();
    let init = read_until_id(&mut reader, 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "srs-mcp");
    assert!(init["result"]["capabilities"]["resources"].is_object());
    assert!(init["result"]["capabilities"]["tools"].is_object());

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    let tools = read_until_id(&mut reader, 2);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "repo_validate",
        "find",
        "record_create",
        "relation_create",
        "note_create",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }

    // Closing stdin ends the session; the process must exit cleanly with no
    // trailing non-protocol output on stdout (ADR-037 carve-out).
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "expected clean exit, got {status}");
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
    for line in rest.lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .expect("post-session stdout must still be protocol-clean JSON");
    }
}

#[test]
fn mcp_serve_without_repo_errors_on_stderr() {
    let dir = tempfile::tempdir().unwrap(); // empty: not an SRS repository

    let output = Command::new(env!("CARGO_BIN_EXE_srs"))
        .args(["mcp", "serve", "--repo"])
        .arg(dir.path().join("nope"))
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must stay protocol-clean, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not an SRS repository") || stderr.contains("Failed to find repository"),
        "stderr should explain the failure: {stderr}"
    );
}
