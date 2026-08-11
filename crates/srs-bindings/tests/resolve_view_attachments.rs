use srs_repository::attachment_service::{
    resolve_document_view_attachments, ResolveDocumentViewAttachmentsInput,
};

const REC_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn minimal_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "repositoryId": "test-resolve-view-attachments",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": REC_ID, "path": format!("records/{REC_ID}.json"), "tier": 2}
            ]
        },
        "data": {
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
fn binding_resolve_document_view_attachments_empty_ids() {
    let srsj = minimal_srsj();
    let store = srs_repository::srsj::open_srsj(&srsj).expect("load store");
    let input = ResolveDocumentViewAttachmentsInput {
        instance_ids: vec![],
    };
    let result = resolve_document_view_attachments(&store, input).expect("resolve ok");
    assert!(result.records.is_empty());
}

#[test]
fn binding_resolve_document_view_attachments_no_source_refs() {
    let srsj = minimal_srsj();
    let store = srs_repository::srsj::open_srsj(&srsj).expect("load store");
    let input = ResolveDocumentViewAttachmentsInput {
        instance_ids: vec![REC_ID.to_string()],
    };
    let result = resolve_document_view_attachments(&store, input).expect("resolve ok");
    // Record has no sourceRefs → no attachments → not included in output
    assert!(result.records.is_empty());
}
