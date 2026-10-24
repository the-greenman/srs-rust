use srs_repository::attachment_service::{
    resolve_composition_attachments, ResolveCompositionAttachmentsInput,
};

const REC_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn minimal_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-resolve-view-attachments",
            "srsVersion": "2.0-draft",
            "namespace": "com.test"
        },
        "data": {
            "package/package.json": package_json(),
            format!("records/{REC_ID}.json"): {
                "instanceId": REC_ID,
                "typeId": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "decision",
                "fieldValues": {}
            }
        }
    })
    .to_string()
}

#[test]
fn binding_resolve_composition_attachments_empty_ids() {
    let srsj = minimal_srsj();
    let store = ladder_open(&srsj).expect("load store");
    let input = ResolveCompositionAttachmentsInput {
        instance_ids: vec![],
    };
    let result = resolve_composition_attachments(&store, input).expect("resolve ok");
    assert!(result.records.is_empty());
}

#[test]
fn binding_resolve_composition_attachments_no_source_refs() {
    let srsj = minimal_srsj();
    let store = ladder_open(&srsj).expect("load store");
    let input = ResolveCompositionAttachmentsInput {
        instance_ids: vec![REC_ID.to_string()],
    };
    let result = resolve_composition_attachments(&store, input).expect("resolve ok");
    // Record has no sourceRefs → no attachments → not included in output
    assert!(result.records.is_empty());
}

/// Fixtures authored below revision 10 are brought to it through the registry ladder
/// (RFC-052 [R7]) before they are opened.
fn ladder_open(
    srsj: &str,
) -> Result<srs_repository::FileStore, srs_repository::error::RepositoryError> {
    srs_repository::srsj::open_srsj(
        &srs_repository::rfc052_root_container_migration_service::ladder_srsj_fixture(srsj)?,
    )
}

/// The package a revision-10 corpus carries (the ladder reads it).
fn package_json() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
        "id": "fixture-pkg", "namespace": "com.test", "name": "primary",
        "version": "1.0.0", "title": "primary", "description": "", "status": "active",
        "createdAt": "2026-01-01T00:00:00Z", "fields": [], "types": []
    })
}
