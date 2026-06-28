use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Resolve the `srs` binary path.
///
/// Resolution order:
/// 1. `SRS_BIN` env var
/// 2. Sibling of the current executable (so `target/debug/srs-gov` finds `target/debug/srs`)
/// 3. `"srs"` from PATH
pub fn srs_bin() -> String {
    if let Ok(v) = std::env::var("SRS_BIN") {
        return v;
    }
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.parent().unwrap_or(Path::new(".")).join("srs");
        if sibling.exists() {
            return sibling.to_string_lossy().into_owned();
        }
    }
    "srs".to_string()
}

/// Run a `srs` subcommand and return its parsed JSON envelope payload.
///
/// If `explain` is true, prints the command and returns `Value::Null` without
/// running it (dry-run mode).
pub fn run_srs(args: &[&str], repo: &str, explain: bool, print_raw: bool) -> Result<Value> {
    run_srs_impl(args, repo, None, explain, print_raw)
}

/// Run a `srs` subcommand with JSON piped to stdin.
pub fn run_srs_write(args: &[&str], repo: &str, stdin_json: &str) -> Result<Value> {
    run_srs_impl(args, repo, Some(stdin_json), false, false)
}

fn run_srs_impl(
    args: &[&str],
    repo: &str,
    stdin_json: Option<&str>,
    explain: bool,
    print_raw: bool,
) -> Result<Value> {
    let bin = srs_bin();
    let mut full: Vec<&str> = vec!["--repo", repo, "--format", "json"];
    full.extend_from_slice(args);

    if explain {
        let cmd_str = std::iter::once(bin.as_str())
            .chain(full.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        println!("  {cmd_str}");
        return Ok(Value::Null);
    }

    let mut child = Command::new(&bin)
        .args(&full)
        .stdin(if stdin_json.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run '{bin}' — set SRS_BIN if it is not on PATH"))?;

    if let Some(json) = stdin_json {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(json.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() && output.stdout.is_empty() {
        bail!(
            "srs exited {:?}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let envelope: Value =
        serde_json::from_slice(&output.stdout).context("srs output was not valid JSON")?;

    if print_raw {
        println!("{}", serde_json::to_string_pretty(&envelope)?);
        return Ok(Value::Null);
    }

    if let Some(false) = envelope.get("ok").and_then(|v| v.as_bool()) {
        let diag = envelope
            .get("diagnostics")
            .and_then(|d| serde_json::to_string_pretty(d).ok())
            .unwrap_or_default();
        bail!("srs command failed:\n{diag}");
    }

    Ok(envelope.get("payload").cloned().unwrap_or(Value::Null))
}
