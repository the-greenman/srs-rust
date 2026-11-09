//! The relational projection (srs-rust#1603) of a fixture repository is SQL
//! that SQLite executes, with every row loaded and no dangling foreign key.
//!
//! Runs the generated SQL through the `sqlite3` binary when one is on PATH
//! (no Rust dependency added for a test); without it, only the shape is
//! asserted.

use srs_projection::{sql_projection, SqlProjectionInput};
use srs_repository::store::FileStore;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../srs-repository/tests/fixtures")
        .join(name)
}

fn run_sqlite(sql: &str) -> Option<String> {
    let which = Command::new("sqlite3").arg("-version").output().ok()?;
    if !which.status.success() {
        return None;
    }
    let script = format!("PRAGMA foreign_keys = ON;\nBEGIN;\n{sql}\nCOMMIT;\nPRAGMA foreign_key_check;\nSELECT 'instances', count(*) FROM instance;\n");
    let mut child = Command::new("sqlite3")
        .arg("-bail")
        .arg(":memory:")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    use std::io::Write;
    child.stdin.take()?.write_all(script.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    assert!(
        out.status.success(),
        "sqlite3 rejected the projection:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn exploded_basic_projects_and_loads() {
    let store = FileStore::new(fixture("exploded-basic"));
    let result = sql_projection(&store, SqlProjectionInput { include_data: true }).unwrap();
    let names: Vec<&str> = result.tables.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"instance"),
        "fixed tables present: {names:?}"
    );
    assert!(
        result.tables.iter().any(|t| t.kind == "type"),
        "at least one Type table: {names:?}"
    );
    assert!(
        result.rows.records + result.rows.notes > 0,
        "rows emitted: {:?}",
        result.rows
    );
    assert!(result.sql.contains("CREATE TABLE"));
    assert!(result.sql.contains("INSERT INTO instance"));

    if let Some(stdout) = run_sqlite(&result.sql) {
        // foreign_key_check prints one line per violation before the count line.
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "foreign_key_check reported violations:\n{stdout}"
        );
        let expected = result.rows.records + result.rows.notes;
        assert_eq!(lines[0], format!("instances|{expected}"));
    }
}

#[test]
fn ddl_only_emits_no_rows() {
    let store = FileStore::new(fixture("exploded-basic"));
    let result = sql_projection(
        &store,
        SqlProjectionInput {
            include_data: false,
        },
    )
    .unwrap();
    assert_eq!(result.rows.records, 0);
    assert!(!result.sql.contains("INSERT INTO"));
}

#[test]
fn projection_is_deterministic() {
    let store = FileStore::new(fixture("exploded-basic"));
    let a = sql_projection(&store, SqlProjectionInput { include_data: true }).unwrap();
    let b = sql_projection(&store, SqlProjectionInput { include_data: true }).unwrap();
    assert_eq!(a.sql, b.sql);
}
