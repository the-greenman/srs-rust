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
fn tool_catalogue_has_all_twenty_three_tools_and_core_owns_the_schemas() {
    let (_dir, mut d) = setup();
    let listed = rpc(&mut d, "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 23);
    assert!(tools
        .iter()
        .all(|t| t["description"].is_string() && t["inputSchema"]["type"] == "object"));
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
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("write guard"), "{text}");
        assert_eq!(epoch(d), before, "rejection must not advance write_epoch");
    }

    fn assert_ok(d: &mut D, name: &str, args: Value) {
        let r = tool(d, name, args);
        assert_eq!(r["result"]["isError"], false, "{r}");
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
