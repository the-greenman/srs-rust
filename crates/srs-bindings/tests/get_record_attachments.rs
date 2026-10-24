use srs_repository::attachment_service::{get_record_attachments, GetRecordAttachmentsInput};

const REC_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

fn minimal_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "dataModelRevision": 2,
            "repositoryId": "test-get-record-attachments",
            "srsVersion": "2.0-draft",
            "namespace": "com.test"
        },
        "data": {
            "package/package.json": package_json(),
            format!("records/{REC_ID}.json"): {
                "instanceId": REC_ID,
                "typeId": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
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
fn get_record_attachments_returns_none_for_missing_id() {
    let srsj = minimal_srsj();
    let store = ladder_open(&srsj).expect("load store");
    let input = GetRecordAttachmentsInput {
        instance_id: "00000000-0000-4000-8000-000000000000".to_string(),
    };
    let result = get_record_attachments(&store, input).expect("service ok");
    assert!(result.is_none());
}

#[test]
fn get_record_attachments_empty_when_no_source_refs() {
    let srsj = minimal_srsj();
    let store = ladder_open(&srsj).expect("load store");
    let input = GetRecordAttachmentsInput {
        instance_id: REC_ID.to_string(),
    };
    let result = get_record_attachments(&store, input)
        .expect("service ok")
        .expect("record found");
    assert_eq!(result.instance_id, REC_ID);
    assert!(result.attachments.is_empty());
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
