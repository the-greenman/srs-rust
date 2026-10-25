//! srs-rust#1444: a published pre-10 document opened through the binding load paths
//! (`SrsRepository::load` = `open_srsj`, `load_archive` = `archive_to_tree`) says it was
//! migrated in memory, reads normally, and refuses every write — through the handle and
//! through an MCP session over it — until the migration is accepted explicitly.
//!
//! Native Rust tests over the services the bindings call (`to_js` panics off-wasm).

use serde_json::{json, Value};
use srs_mcp_core::{McpDispatcher, SrsMcpApplication, MCP_PROTOCOL_VERSION};
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::validate_repository;

/// governance 1.2.2's published seed (revision 7, embedded root), verbatim.
fn seed() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../srs-repository/tests/fixtures/published/governance-1.2.2-seed.srsj"),
    )
    .unwrap()
}

const MIGRATIONS: [&str; 4] = [
    "rfc043-container-entries",
    "rfc046-actor-provenance",
    "rfc052-root-container",
    "rfc051-container-entries",
];

fn note(store: &FileStore) -> Result<(), srs_repository::error::RepositoryError> {
    let input =
        serde_json::from_value(json!({"title":"T","sections":[{"name":"i","content":"x"}]}))
            .unwrap();
    srs_repository::services::create_note(store, input).map(|_| ())
}

fn assert_migrated(store: &FileStore) {
    let n = store.migration_notice().expect("migrated in memory");
    assert_eq!(n.source_revision, 7);
    assert_eq!(n.migrations, MIGRATIONS);

    let report = validate_repository(store).unwrap();
    let notes: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.message.starts_with("migrated-in-memory:"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(
        serde_json::to_value(notes[0].severity).unwrap(),
        json!("info")
    );
    assert!(
        notes[0].message.contains("dataModelRevision 7"),
        "{}",
        notes[0].message
    );
    for id in MIGRATIONS {
        assert!(notes[0].message.contains(id), "{}", notes[0].message);
    }
    assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);

    // Reads work.
    let nav = srs_repository::repository_navigation_service::repository_navigation(store).unwrap();
    assert!(!nav.root_container_id.is_empty());

    // Writes are refused, and nothing is written.
    let before = srs_repository::srsj::to_srsj_string(store).unwrap();
    let err = note(store).unwrap_err();
    assert_eq!(err.code(), "migration-required-before-write", "{err}");
    assert_eq!(srs_repository::srsj::to_srsj_string(store).unwrap(), before);

    // An MCP session over the same handle (`open_mcp_session`) is refused the same way and
    // reports the notice from repo_validate.
    let mut d = McpDispatcher::new(SrsMcpApplication::open(store.clone()).unwrap());
    d.dispatch(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion": MCP_PROTOCOL_VERSION}}));
    let tool = |d: &mut McpDispatcher<_>, name: &str, args: Value| -> Value {
        d.dispatch(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name": name, "arguments": args}}))
            .unwrap()["result"]
            .clone()
    };
    let r = tool(
        &mut d,
        "note_create",
        json!({"title":"T","sections":[{"name":"i","content":"x"}]}),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(
        r["structuredContent"]["code"], "migration-required-before-write",
        "{r}"
    );
    let r = tool(&mut d, "repo_validate", json!({}));
    assert!(r.to_string().contains("migrated-in-memory"), "{r}");
    assert_eq!(srs_repository::srsj::to_srsj_string(store).unwrap(), before);

    // The explicit act (`accept_migration`) lifts the refusal for the handle and its
    // MCP session alike.
    assert_eq!(store.accept_migration().unwrap().migrations, MIGRATIONS);
    assert!(store.migration_notice().is_none());
    note(store).unwrap();
    let r = tool(
        &mut d,
        "note_create",
        json!({"title":"U","sections":[{"name":"i","content":"y"}]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let report = validate_repository(store).unwrap();
    assert!(!report
        .diagnostics
        .iter()
        .any(|d| d.message.starts_with("migrated-in-memory:")));
}

#[test]
fn srsj_load_says_migrated_and_refuses_writes() {
    assert_migrated(&srs_repository::srsj::open_srsj(&seed()).unwrap());
}

#[test]
fn archive_load_says_migrated_and_refuses_writes() {
    // The seed as a `.srs` archive, still at revision 7 (packed without migrating).
    let raw = srs_repository::srsj::open_srsj_for_migration(&seed()).unwrap();
    let bytes = srs_repository::archive_to_vec(&raw).unwrap();
    let store = srs_repository::archive_to_tree(std::io::Cursor::new(bytes)).unwrap();
    assert_migrated(&store);
}

#[test]
fn a_current_document_carries_no_notice() {
    let store = srs_repository::srsj::open_srsj(&seed()).unwrap();
    store.accept_migration();
    let current = srs_repository::srsj::to_srsj_string(&store).unwrap();
    let reopened = srs_repository::srsj::open_srsj(&current).unwrap();
    assert!(reopened.migration_notice().is_none());
    note(&reopened).unwrap();
}
