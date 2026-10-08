//! Full MCP surface through the raw JSON-RPC dispatcher over a real store:
//! the same path the browser session drives (no rmcp, no transport).

use serde_json::{json, Value};
use srs_core::types::blueprint::{Blueprint, TypeRef};
use srs_mcp_core::{McpDispatcher, SrsMcpApplication, MCP_PROTOCOL_VERSION};
use srs_repository::blueprint_service::create_blueprint;
use srs_repository::repository_lifecycle::{
    create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
};
use srs_repository::store::FileStore;

const REPO_ID: &str = "core-surface";

fn setup() -> (
    tempfile::TempDir,
    McpDispatcher<SrsMcpApplication<FileStore>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::new(dir.path());
    create_repository(
        &store,
        &InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: REPO_ID.into(),
                namespace: "com.example.surface".into(),
                srs_version: "2.0".into(),
                title: Some("Surface fixture".into()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg".into(),
                namespace: "com.example.surface".into(),
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
        },
    )
    .unwrap();
    let mut dispatcher = McpDispatcher::new(SrsMcpApplication::open(store).unwrap());
    let init = rpc(
        &mut dispatcher,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL_VERSION }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "srs-mcp");
    (dir, dispatcher)
}

fn rpc(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, method: &str, params: Value) -> Value {
    d.dispatch(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .unwrap()
}

fn tool(d: &mut McpDispatcher<SrsMcpApplication<FileStore>>, name: &str, args: Value) -> Value {
    rpc(d, "tools/call", json!({ "name": name, "arguments": args }))
}

#[test]
fn application_reads_repository_id_from_manifest() {
    let (_dir, d) = setup();
    assert_eq!(d.application().repository_id(), REPO_ID);
}

#[test]
fn tool_catalogue_has_all_thirty_five_tools_and_core_owns_the_schemas() {
    let (_dir, mut d) = setup();
    let listed = rpc(&mut d, "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 35);
    assert!(tools
        .iter()
        .all(|t| t["description"].is_string() && t["inputSchema"]["type"] == "object"));
}

#[test]
fn tool_profile_filters_the_catalogue_and_refuses_the_rest() {
    use srs_mcp_core::tools::ToolProfile;
    let (_dir, mut d) = setup();
    d.application_mut().set_tool_profile(ToolProfile::Context);
    let listed = rpc(&mut d, "tools/list", json!({}));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 12);
    assert!(names.contains(&"repo_validate"));
    assert!(names.contains(&"find") && names.contains(&"read") && names.contains(&"note_create"));
    assert!(!names.contains(&"container_copy"));
    assert!(!names.contains(&"attachment_add") && !names.contains(&"attachment_link"));
    // Advertised tools still work, including `read` (routed outside call_tool).
    let found = tool(&mut d, "find", json!({ "limit": 0 }));
    assert_eq!(found["result"]["isError"], false);
    let read = tool(
        &mut d,
        "read",
        json!({ "uri": format!("srs://{REPO_ID}/map") }),
    );
    assert_eq!(read["result"]["isError"], false, "{read}");
    // A tool outside the profile is refused like an unknown tool, and writes nothing.
    let refused = tool(&mut d, "container_copy", json!({}));
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("'context' tool profile"));
    // Read profile: no write tool is callable.
    d.application_mut().set_tool_profile(ToolProfile::Read);
    let refused = tool(&mut d, "note_create", json!({ "sections": [] }));
    assert_eq!(refused["error"]["code"], -32602);
    let refused = tool(
        &mut d,
        "attachment_add",
        json!({ "fileName": "a", "content": "x" }),
    );
    assert_eq!(refused["error"]["code"], -32602);
}

#[test]
fn package_dependency_tools_use_the_core_service() {
    let (_dir, mut d) = setup();
    // The core package is always installed (RFC-044 Change D item 4).
    let core = "3a000001-0000-4000-a000-000000000001";
    let set = tool(
        &mut d,
        "package_dependency_set",
        json!({ "packageId": core, "version": "1.0.0" }),
    );
    let r = &set["result"]["structuredContent"];
    assert_eq!(r["action"], "added", "{set}");
    assert_eq!(r["dependencies"][0]["namespace"], "com.semanticops.core");
    assert_eq!(r["dependencies"][0]["satisfied"], true);
    // Labels are never guessed: an uninstalled id is a tool error.
    let refused = tool(
        &mut d,
        "package_dependency_set",
        json!({ "packageId": "e0000007-0000-4000-a000-000000000007", "version": "1.0.0" }),
    );
    assert_eq!(refused["result"]["isError"], true);
    let listed = tool(&mut d, "package_dependency_list", json!({}));
    assert_eq!(
        listed["result"]["structuredContent"]["dependencies"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let removed = tool(
        &mut d,
        "package_dependency_remove",
        json!({ "packageId": core }),
    );
    assert_eq!(removed["result"]["structuredContent"]["action"], "removed");
}

/// `package_upgrade` calls the core service: a dry-run plan writes nothing, the real run applies it.
#[test]
fn package_upgrade_tool_uses_the_core_service() {
    let (dir, mut d) = setup();
    let bundle = |version: &str, fields: Value| {
        json!({
            "schemaVersion": "2.0-draft", "packageId": "9a1b0c2d-2222-4aaa-8bbb-000000000009",
            "packageNamespace": "com.example.up", "packageName": "up", "packageVersion": version,
            "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
            "fields": fields, "types": [], "relationTypes": [], "views": [],
            "dependencyRefs": [], "packageDependencies": []
        })
        .to_string()
    };
    let field = |id: &str, name: &str| {
        json!({"id": id, "namespace": "com.example.up", "name": name, "version": 1,
            "description": "d.", "fieldType": {"datatype": "string"},
            "aiGuidance": {"purpose": "p."}, "createdAt": "2026-01-01T00:00:00Z"})
    };
    let (f1, f2) = (
        "9a1b0c2d-0001-4aaa-8bbb-0000000000a1",
        "9a1b0c2d-0001-4aaa-8bbb-0000000000a2",
    );
    let old = bundle("1.0.0", json!([field(f1, "one")]));
    let needs = json!([{"packageId": "e0000007-0000-4000-a000-000000000007",
        "namespace": "com.example.other", "name": "other", "version": "1.0.0"}]);
    let new = bundle("1.1.0", json!([field(f1, "one"), field(f2, "two")]));

    // Not installed: a tool error, nothing written.
    let refused = tool(&mut d, "package_upgrade", json!({ "bundle": new }));
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(refused.to_string().contains("not installed"));

    srs_repository::package_install_service::install_package_bundle_bytes(
        &FileStore::new(dir.path()),
        old.as_bytes(),
        Default::default(),
    )
    .unwrap();
    let dry = tool(
        &mut d,
        "package_upgrade",
        json!({ "bundle": new, "dryRun": true }),
    );
    let r = &dry["result"]["structuredContent"];
    assert_eq!(r["dryRun"], true, "{dry}");
    assert_eq!(r["added"][0]["name"], "two");
    assert_eq!(r["previousVersion"], "1.0.0");
    // dependencyWarnings items are flat (the one shape every adapter emits).
    let mut with_dep: Value = serde_json::from_str(&new).unwrap();
    with_dep["packageDependencies"] = needs;
    let warned = tool(
        &mut d,
        "package_upgrade",
        json!({ "bundle": with_dep.to_string(), "dryRun": true }),
    );
    let w = &warned["result"]["structuredContent"]["dependencyWarnings"][0];
    assert_eq!(w["reason"], "missing", "{warned}");
    for k in [
        "packageId",
        "namespace",
        "name",
        "version",
        "satisfied",
        "candidateVersions",
        "mismatchedLabels",
    ] {
        assert!(w.get(k).is_some(), "{k} in {w}");
    }
    assert!(w.get("entry").is_none());
    let real = tool(&mut d, "package_upgrade", json!({ "bundle": new }));
    let r = &real["result"]["structuredContent"];
    assert_eq!(r["dryRun"], false, "{real}");
    assert_eq!(r["added"][0]["name"], "two");
    assert_eq!(r["upgraded"], true);
    let again = tool(&mut d, "package_upgrade", json!({ "bundle": new }));
    assert_eq!(
        again["result"]["structuredContent"]["added"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn validated_write_then_read_resource_and_find() {
    let (_dir, mut d) = setup();
    let created = tool(
        &mut d,
        "note_create",
        json!({ "title": "Browser note", "sections": [{ "name": "body", "content": "hello" }] }),
    );
    assert_eq!(created["result"]["isError"], false, "{created}");
    let id = created["result"]["structuredContent"]["instanceId"]
        .as_str()
        .unwrap()
        .to_string();

    let map = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/map") }),
    );
    let content = &map["result"]["contents"][0];
    assert_eq!(content["mimeType"], "application/json");
    assert!(serde_json::from_str::<Value>(content["text"].as_str().unwrap()).is_ok());

    let tree = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/tree/{id}") }),
    );
    assert!(tree["result"]["contents"][0]["text"].is_string(), "{tree}");

    let validate = tool(&mut d, "repo_validate", json!({}));
    assert_eq!(validate["result"]["isError"], false);
}

#[test]
fn service_rejection_is_a_tool_result_not_a_protocol_error() {
    let (_dir, mut d) = setup();
    let rejected = tool(
        &mut d,
        "record_create",
        json!({ "type": "no.such/type", "fieldValues": {} }),
    );
    assert!(rejected.get("error").is_none(), "{rejected}");
    assert_eq!(rejected["result"]["isError"], true);

    assert_eq!(
        tool(&mut d, "no_such_tool", json!({}))["error"]["code"],
        -32602
    );
    assert_eq!(
        tool(&mut d, "find", json!({ "bogus": 1 }))["error"]["code"],
        -32602,
        "unknown argument keys are invalid params"
    );
    assert_eq!(
        rpc(&mut d, "bogus/method", json!({}))["error"]["code"],
        -32601
    );
}

#[test]
fn resource_read_errors_preserve_mcp_codes() {
    let (_dir, mut d) = setup();
    let missing = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": format!("srs://{REPO_ID}/record/missing") }),
    );
    assert_eq!(missing["error"]["code"], -32002);
    let foreign = rpc(
        &mut d,
        "resources/read",
        json!({ "uri": "srs://other/map" }),
    );
    assert_eq!(foreign["error"]["code"], -32602);
    assert_eq!(
        rpc(&mut d, "resources/read", json!({}))["error"]["code"],
        -32602
    );
}

fn blueprint(name: &str) -> Blueprint {
    Blueprint {
        schema: None,
        id: String::new(),
        namespace: "test.ns".into(),
        name: name.into(),
        version: 1,
        description: format!("{name} description"),
        root_types: vec![TypeRef {
            type_id: "placeholder-type-id".into(),
            type_version: None,
        }],
        structure: vec![],
        required_types: vec![],
        ai_guidance: None,
        tags: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        lineage: None,
        provenance: None,
    }
}

#[test]
fn prompts_list_and_get_one_per_blueprint() {
    let (dir, mut d) = setup();
    let created = create_blueprint(&FileStore::new(dir.path()), blueprint("my-bp"), None).unwrap();
    let id = created.blueprint.id;

    let listed = rpc(&mut d, "prompts/list", json!({}));
    let prompts = listed["result"]["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0]["name"], id);
    assert!(prompts[0]["description"]
        .as_str()
        .unwrap()
        .contains("test.ns/my-bp"));

    let got = rpc(&mut d, "prompts/get", json!({ "name": id }));
    let message = &got["result"]["messages"][0];
    assert_eq!(message["role"], "user");
    assert_eq!(message["content"]["type"], "text");
    assert!(message["content"]["text"]
        .as_str()
        .unwrap()
        .contains("my-bp"));
}

#[test]
fn prompt_errors_use_invalid_params() {
    let (_dir, mut d) = setup();
    let unknown = rpc(&mut d, "prompts/get", json!({ "name": "no-such-id" }));
    assert_eq!(unknown["error"]["code"], -32602);
    assert!(unknown["error"]["message"]
        .as_str()
        .unwrap()
        .contains("prompt not found"));
    let with_args = rpc(
        &mut d,
        "prompts/get",
        json!({ "name": "x", "arguments": { "foo": "bar" } }),
    );
    assert_eq!(with_args["error"]["code"], -32602);
}

#[test]
fn container_create_accepts_the_complete_field_set_and_rejects_unknown_keys() {
    let (_dir, mut d) = setup();
    let listed = rpc(&mut d, "tools/list", json!({}));
    let schema = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "container_create")
        .unwrap()["inputSchema"]
        .clone();
    for key in [
        "childContainerIds",
        "meta",
        "namespace",
        "name",
        "createdAt",
    ] {
        assert!(schema["properties"][key].is_object(), "schema lacks {key}");
    }

    let child = tool(&mut d, "container_create", json!({ "title": "Child" }));
    let child_id = child["result"]["structuredContent"]["containerId"].clone();
    let parent = tool(
        &mut d,
        "container_create",
        json!({ "title": "Parent", "childContainerIds": [child_id], "meta": { "x": 1 } }),
    );
    assert_eq!(parent["result"]["isError"], false, "{parent}");
    let c = &parent["result"]["structuredContent"];
    assert_eq!(c["childContainerIds"], json!([child_id]));
    assert_eq!(c["meta"], json!({ "x": 1 }));

    assert_eq!(
        tool(
            &mut d,
            "container_create",
            json!({ "title": "x", "bogus": 1 })
        )["error"]["code"],
        -32602
    );
}

// ── Session write guard (srs-rust#1165) ──────────────────────────────────────

mod write_guard {
    use super::*;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_mcp_core::guard::WriteGuard;
    use srs_repository::package_service;

    const NS: &str = "com.example.surface";
    type D = McpDispatcher<SrsMcpApplication<FileStore>>;

    fn field(name: &str) -> Field {
        Field {
            schema: None,
            id: format!("guard-field-{name}"),
            namespace: NS.into(),
            name: name.into(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "test".into(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn paragraph_type(fields: &[&str]) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: format!("guard-type-{}", fields.len()),
            namespace: NS.into(),
            name: format!("para{}", fields.len()),
            version: 1,
            description: String::new(),
            fields: fields
                .iter()
                .enumerate()
                .map(|(i, f)| FieldAssignment {
                    field_id: format!("guard-field-{f}"),
                    order: i as u32,
                    required: false,
                    display_label: None,
                    description: None,
                })
                .collect(),
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            lineage: None,
            provenance: None,
        }
    }

    fn guarded() -> (tempfile::TempDir, D) {
        let (dir, d) = setup();
        let s = d.application().store();
        package_service::create_field(s, field("body")).unwrap();
        package_service::create_field(s, field("paragraph_title")).unwrap();
        package_service::create_type(s, paragraph_type(&["body", "paragraph_title"])).unwrap();
        // Same Fields minus the fill-only one: effectively locked.
        package_service::create_type(s, paragraph_type(&["body"])).unwrap();
        (dir, d)
    }

    fn create(d: &mut D, ty: &str, fv: Value, container: Option<&str>) -> String {
        let r = tool(
            d,
            "record_create",
            json!({ "type": ty, "fieldValues": fv, "containerId": container }),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        r["result"]["structuredContent"]["instanceId"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn guard(d: &mut D, g: Value) {
        let g: WriteGuard = serde_json::from_value(g).unwrap();
        d.application_mut().set_write_guard(Some(g));
    }

    fn epoch(d: &D) -> u64 {
        d.application().store().write_epoch()
    }

    /// The call is a tool error naming the guard and the epoch did not move.
    fn assert_rejected(d: &mut D, name: &str, args: Value) {
        let before = epoch(d);
        let r = tool(d, name, args);
        assert!(
            r.get("error").is_none(),
            "must not be a protocol error: {r}"
        );
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert_eq!(
            r["result"]["structuredContent"]["code"], "write-guard-rejected",
            "{r}"
        );
        assert_eq!(epoch(d), before, "rejection must not advance write_epoch");
    }

    fn assert_ok(d: &mut D, name: &str, args: Value) {
        let r = tool(d, name, args);
        assert_eq!(r["result"]["isError"], false, "{r}");
    }

    #[test]
    fn attachment_add_rejected_by_policy_writes_nothing() {
        let (dir, mut d) = guarded();
        // Install the base repo_settings policy shape through the same service writes.
        let s = d.application().store();
        let mut f = field("max_per_file_bytes");
        f.namespace = "com.semanticops.base".into();
        f.field_type = srs_core::types::field::FieldType::number();
        package_service::create_field(s, f).unwrap();
        let mut t = paragraph_type(&["max_per_file_bytes"]);
        t.id = "guard-type-policy".into();
        t.namespace = "com.semanticops.base".into();
        t.name = "repo_settings".into();
        t.fields[0].field_id = "guard-field-max_per_file_bytes".into();
        package_service::create_type(s, t).unwrap();
        create(
            &mut d,
            "com.semanticops.base/repo_settings",
            json!({ "max_per_file_bytes": 3 }),
            None,
        );
        let r = tool(
            &mut d,
            "attachment_add",
            json!({ "fileName": "big.txt", "content": "hello" }),
        );
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("max_per_file_bytes"));
        assert!(!dir.path().join("source-documents/big.txt").exists());
        assert_ok(
            &mut d,
            "attachment_add",
            json!({ "fileName": "ok.txt", "content": "hi" }),
        );
    }

    #[test]
    fn attachment_add_and_link_round_trip_and_guard() {
        let (dir, mut d) = guarded();
        let id = create(
            &mut d,
            "com.example.surface/para1",
            json!({ "body": "t" }),
            None,
        );
        // Text and base64 store identical bytes.
        let a = tool(
            &mut d,
            "attachment_add",
            json!({ "fileName": "a.txt", "content": "hello", "title": "A" }),
        );
        assert_eq!(a["result"]["isError"], false, "{a}");
        let b = tool(
            &mut d,
            "attachment_add",
            json!({ "fileName": "b.txt", "contentBase64": "aGVsbG8=" }),
        );
        assert_eq!(b["result"]["isError"], false, "{b}");
        let docs = dir.path().join("source-documents");
        assert_eq!(std::fs::read(docs.join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(docs.join("b.txt")).unwrap(), b"hello");
        // Exactly one content source; bad base64 is an invalid-params protocol error.
        for bad in [
            json!({ "fileName": "c.txt" }),
            json!({ "fileName": "c.txt", "content": "x", "contentBase64": "eA==" }),
            json!({ "fileName": "c.txt", "contentBase64": "!!" }),
        ] {
            assert!(tool(&mut d, "attachment_add", bad).get("error").is_some());
        }
        assert!(!docs.join("c.txt").exists());
        // Link, then a duplicate and an unknown document are tool errors.
        let doc = a["result"]["structuredContent"]["documentId"]
            .as_str()
            .unwrap();
        let link = json!({ "instanceId": id, "documentId": doc });
        assert_ok(&mut d, "attachment_link", link.clone());
        let dup = tool(&mut d, "attachment_link", link.clone());
        assert_eq!(dup["result"]["isError"], true, "{dup}");
        let unknown = tool(
            &mut d,
            "attachment_link",
            json!({ "instanceId": id, "documentId": "nope" }),
        );
        assert_eq!(unknown["result"]["isError"], true, "{unknown}");
        // A guarded record rejects linking; adding a document is still allowed.
        guard(&mut d, json!({ "instanceIds": [id] }));
        let other = b["result"]["structuredContent"]["documentId"]
            .as_str()
            .unwrap();
        assert_rejected(
            &mut d,
            "attachment_link",
            json!({ "instanceId": id, "documentId": other }),
        );
        assert_ok(
            &mut d,
            "attachment_add",
            json!({ "fileName": "d.txt", "content": "x" }),
        );
    }

    #[test]
    fn guard_rejection_is_coded() {
        let (_dir, mut d) = guarded();
        let id = create(
            &mut d,
            "com.example.surface/para2",
            json!({ "body": "text" }),
            None,
        );
        guard(&mut d, json!({ "instanceIds": [id] }));
        let r = tool(
            &mut d,
            "record_update",
            json!({ "instanceId": id, "fieldValues": { "body": "x" } }),
        );
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert_eq!(
            r["result"]["structuredContent"]["code"],
            "write-guard-rejected"
        );
        assert_eq!(
            r["result"]["content"][0]["text"],
            r["result"]["structuredContent"]["message"]
        );
    }

    #[test]
    fn fill_only_field_writable_once_other_fields_never() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let id = create(&mut d, ty, json!({ "body": "text" }), None);
        guard(
            &mut d,
            json!({ "instanceIds": [id], "fillOnlyFields": ["paragraph_title"] }),
        );
        let upd = |fv: Value| json!({ "instanceId": id, "fieldValues": fv });
        // Changing text, or dropping it, is rejected.
        assert_rejected(&mut d, "record_update", upd(json!({ "body": "edited" })));
        assert_rejected(
            &mut d,
            "record_update",
            upd(json!({ "paragraph_title": "T" })),
        );
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": id, "fieldValues": { "body": "text" }, "tags": ["x"] }),
        );
        // Unchanged body plus a first title is allowed...
        assert_ok(
            &mut d,
            "record_update",
            upd(json!({ "body": "text", "paragraph_title": "Intro" })),
        );
        // ...writing the current value back is not a change...
        assert_ok(
            &mut d,
            "record_update",
            upd(json!({ "body": "text", "paragraph_title": "Intro" })),
        );
        // ...but once set it cannot be changed.
        assert_rejected(
            &mut d,
            "record_update",
            upd(json!({ "body": "text", "paragraph_title": "Other" })),
        );
    }

    #[test]
    fn empty_string_and_null_count_as_unset() {
        let (_dir, mut d) = guarded();
        let id = create(
            &mut d,
            "com.example.surface/para2",
            json!({ "body": "t", "paragraph_title": "" }),
            None,
        );
        guard(
            &mut d,
            json!({ "instanceIds": [id], "fillOnlyFields": ["paragraph_title"] }),
        );
        assert_ok(
            &mut d,
            "record_update",
            json!({ "instanceId": id, "fieldValues": { "body": "t", "paragraph_title": "Filled" } }),
        );
    }

    #[test]
    fn locked_record_and_lifecycle_tools() {
        let (_dir, mut d) = guarded();
        let id = create(
            &mut d,
            "com.example.surface/para1",
            json!({ "body": "t" }),
            None,
        );
        guard(
            &mut d,
            json!({ "instanceIds": [id], "fillOnlyFields": ["paragraph_title"] }),
        );
        // Type has no fill-only field: nothing on it can change.
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": id, "fieldValues": { "body": "edited" } }),
        );
        assert_rejected(
            &mut d,
            "record_transition",
            json!({ "instanceId": id, "to": "active" }),
        );
        assert_rejected(
            &mut d,
            "record_successor",
            json!({ "predecessorId": id, "relationType": "supersedes", "fieldValues": { "body": "n" } }),
        );
    }

    #[test]
    fn container_membership_is_resolved_at_write_time_and_container_writes_rejected() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let c = tool(&mut d, "container_create", json!({ "title": "Essay" }));
        let cid = c["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        let early = create(&mut d, ty, json!({ "body": "a" }), Some(&cid));
        guard(
            &mut d,
            json!({ "containerIds": [cid], "fillOnlyFields": ["paragraph_title"] }),
        );
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": early, "fieldValues": { "body": "x" } }),
        );
        // Added to the container after the guard was set: still covered.
        let loose = create(&mut d, ty, json!({ "body": "b" }), None);
        let before = epoch(&d);
        // (adding is a container write, so do it directly on the store, as the human UI would)
        srs_repository::container_service::add_member(
            d.application().store(),
            &cid,
            &loose,
            None,
            None,
        )
        .unwrap();
        assert!(epoch(&d) > before);
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": loose, "fieldValues": { "body": "x" } }),
        );
        // Container structure writes.
        let other = create(&mut d, ty, json!({ "body": "c" }), None);
        assert_rejected(
            &mut d,
            "container_member_add",
            json!({ "containerId": cid, "instanceId": other }),
        );
        assert_rejected(
            &mut d,
            "container_member_remove",
            json!({ "containerId": cid, "instanceId": early }),
        );
        assert_rejected(
            &mut d,
            "container_member_move",
            json!({ "containerId": cid, "instanceId": early, "position": 0 }),
        );
        assert_rejected(
            &mut d,
            "container_member_repair",
            json!({ "containerId": cid }),
        );
        assert_rejected(
            &mut d,
            "record_create",
            json!({ "type": ty, "fieldValues": {}, "containerId": cid }),
        );
        // A guarded record may join an unguarded container: it is not modified.
        let c2 = tool(&mut d, "container_create", json!({ "title": "Other" }));
        let c2id = c2["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ok(
            &mut d,
            "container_member_add",
            json!({ "containerId": c2id, "instanceId": early }),
        );
    }

    #[test]
    fn relations_and_new_instances_are_allowed_and_clear_lifts_the_guard() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let a = create(&mut d, ty, json!({ "body": "a" }), None);
        let b = create(&mut d, ty, json!({ "body": "b" }), None);
        guard(&mut d, json!({ "instanceIds": [a, b] }));
        assert_ok(
            &mut d,
            "relation_create",
            json!({ "relationType": "refines", "sourceInstanceId": a, "targetInstanceId": b }),
        );
        assert_ok(
            &mut d,
            "record_create",
            json!({ "type": ty, "fieldValues": { "body": "comment" } }),
        );
        assert_ok(
            &mut d,
            "note_create",
            json!({ "title": "feedback", "sections": [{ "name": "body", "content": "hi" }] }),
        );
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": a, "fieldValues": { "body": "x" } }),
        );
        d.application_mut().set_write_guard(None);
        assert_ok(
            &mut d,
            "record_update",
            json!({ "instanceId": a, "fieldValues": { "body": "x" } }),
        );
    }

    #[test]
    fn fork_and_copy_respect_the_guard() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let c = tool(&mut d, "container_create", json!({ "title": "Essay" }));
        let cid = c["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        let p = create(&mut d, ty, json!({ "body": "a" }), Some(&cid));
        // A second, unguarded container shares the paragraph.
        let c2 = tool(&mut d, "container_create", json!({ "title": "Other" }));
        let cid2 = c2["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        srs_repository::container_service::add_member(
            d.application().store(),
            &cid2,
            &p,
            None,
            None,
        )
        .unwrap();
        guard(&mut d, json!({ "containerIds": [cid] }));
        // Fork inside the guarded container: rejected. Inside the unguarded one: allowed.
        assert_rejected(
            &mut d,
            "record_fork",
            json!({ "containerId": cid, "instanceId": p }),
        );
        // (p is a member of the guarded container, so even the unguarded container's fork
        // creates only a NEW record; the original is untouched.)
        assert_ok(
            &mut d,
            "record_fork",
            json!({ "containerId": cid2, "instanceId": p }),
        );
        let m2 = srs_repository::container_service::list_members(d.application().store(), &cid2)
            .unwrap();
        assert_ne!(m2, vec![p.clone()]);
        let m1 =
            srs_repository::container_service::list_members(d.application().store(), &cid).unwrap();
        assert_eq!(m1, vec![p.clone()]);
        // Copy of a guarded source is allowed; copy onto a guarded id is rejected.
        let r = tool(
            &mut d,
            "container_copy",
            json!({ "sourceContainerId": cid }),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_rejected(
            &mut d,
            "container_copy",
            json!({ "sourceContainerId": cid, "containerId": cid }),
        );
    }

    /// #1246: a guarded session retracts only relations it created; unguarded is unrestricted.
    #[test]
    fn relation_delete_is_own_only_when_guarded() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let a = create(&mut d, ty, json!({ "body": "a" }), None);
        let b = create(&mut d, ty, json!({ "body": "b" }), None);
        let rel = |d: &mut D| -> String {
            let r = tool(
                d,
                "relation_create",
                json!({ "relationType": "refines", "sourceInstanceId": a, "targetInstanceId": b, "createdAt": "2026-10-04T00:00:00Z" }),
            );
            assert_eq!(r["result"]["isError"], false, "{r}");
            r["result"]["structuredContent"]["relationId"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let actor = |d: &D, id: &str| {
            d.application()
                .set_session_actor(Some(json!({ "kind": "ai", "id": id })))
        };
        let bare = rel(&mut d); // no session actor: no createdBy
                                // createdAt is stamped when the caller omits it (#1246).
        let r = tool(
            &mut d,
            "relation_create",
            json!({ "relationType": "refines", "sourceInstanceId": b, "targetInstanceId": a }),
        );
        assert!(
            r["result"]["structuredContent"]["createdAt"].is_string(),
            "{r}"
        );
        let stamped = r["result"]["structuredContent"]["relationId"]
            .as_str()
            .unwrap()
            .to_string();
        actor(&d, "agent-1");
        let mine = rel(&mut d);
        actor(&d, "agent-2");
        let theirs = rel(&mut d);
        actor(&d, "agent-1");
        guard(&mut d, json!({ "instanceIds": [a] }));

        // The context read carries provenance.
        let ctx = srs_repository::context_query_service::get_record_context(
            d.application().store(),
            srs_repository::context_query_service::RecordContextQuery {
                record_id: a.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap();
        let by = |id: &str| {
            let r = ctx
                .relations
                .iter()
                .find(|r| r.relation.relation_id == id)
                .unwrap();
            (
                r.created_at.is_some(),
                r.created_by.as_ref().map(|c| c.id.clone()),
            )
        };
        assert_eq!(by(&mine), (true, Some("agent-1".into())));
        assert_eq!(by(&theirs).1, Some("agent-2".into()));
        assert_eq!(by(&bare).1, None);
        assert!(by(&stamped).0);

        assert_rejected(&mut d, "relation_delete", json!({ "relationId": theirs }));
        assert_rejected(&mut d, "relation_delete", json!({ "relationId": bare }));
        d.application_mut().take_write_summary();
        assert_ok(&mut d, "relation_delete", json!({ "relationId": mine }));
        let s = d.application_mut().take_write_summary().unwrap();
        assert_eq!(s.tool, "relation_delete");
        let e = &s.changed[0];
        assert_eq!(e.id, mine);
        assert_eq!(e.kind, srs_repository::ChangeKind::Deleted);
        assert_eq!(e.source_instance_id.as_deref(), Some(a.as_str()));
        assert_eq!(e.target_instance_id.as_deref(), Some(b.as_str()));

        d.application_mut().set_write_guard(None);
        assert_ok(&mut d, "relation_delete", json!({ "relationId": theirs }));
        assert_ok(&mut d, "relation_delete", json!({ "relationId": bare }));
    }

    /// ADR-049: the summary lists every entity record_create / record_fork /
    /// container_copy wrote, and matches write_epoch movement.
    #[test]
    fn write_summary_covers_create_fork_and_copy() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let c = tool(&mut d, "container_create", json!({ "title": "Essay" }));
        let cid = c["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        d.application_mut().take_write_summary();
        let p = create(&mut d, ty, json!({ "body": "a" }), Some(&cid));
        let s = d.application_mut().take_write_summary().unwrap();
        assert_eq!(s.tool, "record_create");
        let has = |s: &srs_mcp_core::WriteSummary, t: &str, id: &str, k: &str| {
            let v = serde_json::to_value(&s.changed).unwrap();
            v.as_array()
                .unwrap()
                .iter()
                .any(|c| c["target"] == t && c["kind"] == k && (id.is_empty() || c["id"] == id))
        };
        assert!(has(&s, "instance", &p, "created"), "{s:?}");
        assert!(has(&s, "container", &cid, "updated"), "{s:?}");

        let before = epoch(&d);
        assert_ok(
            &mut d,
            "record_fork",
            json!({ "containerId": cid, "instanceId": p }),
        );
        assert!(epoch(&d) > before);
        let s = d.application_mut().take_write_summary().unwrap();
        assert_eq!(s.tool, "record_fork");
        assert!(has(&s, "instance", "", "created"), "{s:?}");

        assert_ok(
            &mut d,
            "container_copy",
            json!({ "sourceContainerId": cid }),
        );
        let s = d.application_mut().take_write_summary().unwrap();
        assert_eq!(s.tool, "container_copy");
        // an anchor-less copy creates the container only (members are shared); see write_summary_covers_anchored_container_copy
        assert!(has(&s, "container", "", "created"), "{s:?}");
        assert!(!has(&s, "instance", "", "created"), "{s:?}");

        // a rejected write leaves no summary
        guard(&mut d, json!({ "containerIds": [cid] }));
        assert_rejected(
            &mut d,
            "record_fork",
            json!({ "containerId": cid, "instanceId": p }),
        );
        assert!(d.application_mut().take_write_summary().is_none());
    }

    /// ADR-049 / #1136: copying a container WITH an anchor forks the anchor
    /// (new record + derived-from relation); the summary must report both.
    #[test]
    fn write_summary_covers_anchored_container_copy() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let anchor = create(&mut d, ty, json!({ "body": "a" }), None);
        let c = tool(
            &mut d,
            "container_create",
            json!({ "title": "Essay", "anchorInstanceId": anchor,
                    "memberInstanceIds": [{ "instanceId": anchor }] }),
        );
        assert_eq!(c["result"]["isError"], false, "{c}");
        let cid = c["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        d.application_mut().take_write_summary();
        let r = tool(
            &mut d,
            "container_copy",
            json!({ "sourceContainerId": cid }),
        );
        assert_eq!(r["result"]["isError"], false, "{r}");
        let s = d.application_mut().take_write_summary().unwrap();
        let v = serde_json::to_value(&s.changed).unwrap();
        let created = |t: &str| {
            v.as_array()
                .unwrap()
                .iter()
                .filter(|c| c["target"] == t && c["kind"] == "created")
                .count()
        };
        assert_eq!(created("container"), 1, "{s:?}");
        assert_eq!(created("instance"), 1, "forked anchor: {s:?}");
        assert_eq!(created("relation"), 1, "derived-from: {s:?}");
        assert_eq!(s.changed.len(), 3, "{s:?}");
    }

    #[test]
    fn container_create_cannot_overwrite_a_guarded_container() {
        let (_dir, mut d) = guarded();
        let c = tool(&mut d, "container_create", json!({ "title": "Essay" }));
        let cid = c["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        let p = create(
            &mut d,
            "com.example.surface/para2",
            json!({ "body": "a" }),
            Some(&cid),
        );
        guard(&mut d, json!({ "containerIds": [cid] }));
        let before = epoch(&d);
        let r = tool(
            &mut d,
            "container_create",
            json!({ "containerId": cid, "title": "Empty" }),
        );
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert_eq!(epoch(&d), before);
        let members =
            srs_repository::container_service::list_members(d.application().store(), &cid).unwrap();
        assert_eq!(members, vec![p]);
    }

    #[test]
    fn child_containers_of_a_guarded_container_are_guarded() {
        let (_dir, mut d) = guarded();
        let ty = "com.example.surface/para2";
        let child = tool(&mut d, "container_create", json!({ "title": "Child" }));
        let child = child["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        let p = create(&mut d, ty, json!({ "body": "a" }), Some(&child));
        let parent = tool(
            &mut d,
            "container_create",
            json!({ "title": "Parent", "childContainerIds": [child] }),
        );
        let parent = parent["result"]["structuredContent"]["containerId"]
            .as_str()
            .unwrap()
            .to_string();
        guard(&mut d, json!({ "containerIds": [parent] }));
        assert_rejected(
            &mut d,
            "container_member_remove",
            json!({ "containerId": child, "instanceId": p }),
        );
        assert_rejected(
            &mut d,
            "container_member_move",
            json!({ "containerId": child, "instanceId": p, "position": 0 }),
        );
        assert_rejected(
            &mut d,
            "record_update",
            json!({ "instanceId": p, "fieldValues": { "body": "x" } }),
        );
    }

    #[test]
    fn field_meta_meta_tags_and_type_version_cannot_be_rewritten() {
        let (_dir, mut d) = guarded();
        let r = tool(
            &mut d,
            "record_create",
            json!({ "type": "com.example.surface/para2", "fieldValues": { "body": "a" },
                    "fieldMeta": { "body": { "source": "human" } } }),
        );
        let id = r["result"]["structuredContent"]["instanceId"]
            .as_str()
            .unwrap()
            .to_string();
        guard(
            &mut d,
            json!({ "instanceIds": [id], "fillOnlyFields": ["paragraph_title"] }),
        );
        let base = |extra: Value| {
            let mut a = json!({ "instanceId": id, "fieldValues": { "body": "a" } });
            a.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            a
        };
        assert_rejected(&mut d, "record_update", base(json!({ "fieldMeta": {} })));
        assert_rejected(&mut d, "record_update", base(json!({ "tags": ["x"] })));
        assert_rejected(&mut d, "record_update", base(json!({ "meta": { "k": 1 } })));
        assert_rejected(&mut d, "record_update", base(json!({ "typeVersion": 1 })));
    }

    #[test]
    fn record_successor_of_a_guarded_record_is_rejected() {
        let (_dir, mut d) = guarded();
        let id = create(
            &mut d,
            "com.example.surface/para2",
            json!({ "body": "a" }),
            None,
        );
        guard(&mut d, json!({ "instanceIds": [id] }));
        assert_rejected(
            &mut d,
            "record_successor",
            json!({ "predecessorId": id, "relationType": "supersedes", "fieldValues": { "body": "n" } }),
        );
    }
}

mod read_tool {
    use super::*;

    fn uri(path: &str) -> String {
        format!("srs://{REPO_ID}/{path}")
    }

    #[test]
    fn read_equals_resources_read_and_errors_as_there() {
        let (_dir, mut d) = setup();
        for path in [
            "map",
            "navigation",
            "agent-index",
            "tree",
            "protocol",
            "relation-types",
        ] {
            let direct = rpc(&mut d, "resources/read", json!({ "uri": uri(path) }));
            let via = tool(&mut d, "read", json!({ "uri": uri(path) }));
            assert_eq!(
                via["result"]["content"][0]["text"], direct["result"]["contents"][0]["text"],
                "{path}"
            );
            assert_eq!(via["result"]["structuredContent"]["truncated"], false);
            assert_eq!(
                via["result"]["structuredContent"]["text"],
                via["result"]["content"][0]["text"]
            );
            assert_eq!(
                via["result"]["structuredContent"]["mimeType"],
                direct["result"]["contents"][0]["mimeType"]
            );
        }
        let missing = uri("record/00000000-0000-4000-8000-000000000000");
        let direct = rpc(&mut d, "resources/read", json!({ "uri": missing }));
        let via = tool(&mut d, "read", json!({ "uri": missing }));
        assert_eq!(via["error"], direct["error"]);
        let wrong = tool(&mut d, "read", json!({ "uri": "srs://other/map" }));
        assert_eq!(wrong["error"]["code"], -32602);
    }

    #[test]
    fn read_is_not_blocked_by_the_write_guard() {
        let (_dir, mut d) = setup();
        d.application_mut()
            .set_write_guard(Some(srs_mcp_core::guard::WriteGuard {
                container_ids: vec!["x".into()],
                ..Default::default()
            }));
        let via = tool(&mut d, "read", json!({ "uri": uri("map") }));
        assert_eq!(via["result"]["isError"], false, "{via}");
    }

    #[test]
    fn oversize_is_cut_on_a_char_boundary_with_a_notice() {
        let (_dir, d) = setup();
        let args = json!({ "uri": uri("map") });
        let args = args.as_object().cloned();
        // 1 lands inside a multi-byte char only if the map has one; any cap must stay valid UTF-8.
        for cap in [1usize, 7, 100] {
            let via = srs_mcp_core::tools::read_tool_capped(
                d.application().store(),
                REPO_ID,
                args.clone(),
                cap,
            )
            .unwrap();
            let sc = &via["structuredContent"];
            assert_eq!(sc["truncated"], true);
            assert!(sc["totalBytes"].as_u64().unwrap() > cap as u64);
            assert!(sc["shownBytes"].as_u64().unwrap() <= cap as u64);
            let text = via["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("[truncated: showing"));
            assert_eq!(sc["text"], via["content"][0]["text"]);
        }
    }
}

/// `package_upgrade` priorBundles / adopt (#1325): a real no-reference-copy conflict is adopted.
#[test]
fn package_upgrade_tool_adopts_a_no_reference_copy_conflict() {
    let (dir, mut d) = setup();
    let f1 = "9a1b0c2d-0001-4aaa-8bbb-0000000000b1";
    let bundle = |version: &str, desc: &str| {
        json!({
            "schemaVersion": "2.0-draft", "packageId": "9a1b0c2d-2222-4aaa-8bbb-00000000000a",
            "packageNamespace": "com.example.ad", "packageName": "ad", "packageVersion": version,
            "dataModelRevision": 9, "publishedAt": "2026-10-03T00:00:00Z", "mode": "bundled",
            "fields": [{"id": f1, "namespace": "com.example.ad", "name": "one", "version": 1,
                "description": desc, "fieldType": {"datatype": "string"},
                "aiGuidance": {"purpose": "p."}, "createdAt": "2026-01-01T00:00:00Z"}],
            "types": [], "relationTypes": [], "views": [],
            "dependencyRefs": [], "packageDependencies": []
        })
        .to_string()
    };
    let (old, new) = (bundle("1.0.0", "d."), bundle("1.1.0", "Changed."));
    srs_repository::package_install_service::install_package_bundle_bytes(
        &FileStore::new(dir.path()),
        old.as_bytes(),
        Default::default(),
    )
    .unwrap();
    std::fs::remove_dir_all(dir.path().join("packages/ad/.srs-import/refs")).unwrap();
    let bare = tool(
        &mut d,
        "package_upgrade",
        json!({ "bundle": new, "dryRun": true }),
    );
    let r = &bare["result"]["structuredContent"];
    assert_eq!(
        r["conflicts"][0]["conflictKind"], "no-reference-copy",
        "{bare}"
    );
    let proven = tool(
        &mut d,
        "package_upgrade",
        json!({ "bundle": new, "dryRun": true, "priorBundles": [old] }),
    );
    assert_eq!(
        proven["result"]["structuredContent"]["updated"][0]["provenBy"], "1.0.0",
        "{proven}"
    );
    let adopted = tool(
        &mut d,
        "package_upgrade",
        json!({ "bundle": new, "adopt": [f1] }),
    );
    let r = &adopted["result"]["structuredContent"];
    assert_eq!(r["adopted"][0]["id"], f1, "{adopted}");
    assert_eq!(r["conflicts"].as_array().unwrap().len(), 0);
}
