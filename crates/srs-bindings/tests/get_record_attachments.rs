use srs_repository::attachment_service::{get_record_attachments, GetRecordAttachmentsInput};
use srs_repository::JsonStore;

const REC_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

fn minimal_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-get-record-attachments",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [
                {"instanceId": REC_ID, "path": format!("records/{REC_ID}.json"), "tier": 2}
            ]
        },
        "data": {
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
    let store = JsonStore::from_srsj(&srsj).expect("load store");
    let input = GetRecordAttachmentsInput {
        instance_id: "00000000-0000-4000-8000-000000000000".to_string(),
    };
    let result = get_record_attachments(&store, input).expect("service ok");
    assert!(result.is_none());
}

#[test]
fn get_record_attachments_empty_when_no_source_refs() {
    let srsj = minimal_srsj();
    let store = JsonStore::from_srsj(&srsj).expect("load store");
    let input = GetRecordAttachmentsInput {
        instance_id: REC_ID.to_string(),
    };
    let result = get_record_attachments(&store, input)
        .expect("service ok")
        .expect("record found");
    assert_eq!(result.instance_id, REC_ID);
    assert!(result.attachments.is_empty());
}
