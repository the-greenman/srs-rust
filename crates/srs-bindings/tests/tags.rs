use srs_repository::{tag_service, JsonStore};

fn gallery_store() -> JsonStore {
    let srsj = include_str!("fixtures/gallery.srsj");
    JsonStore::from_srsj(srsj).expect("gallery srsj must load")
}

#[test]
fn list_tags_empty_on_gallery() {
    let store = gallery_store();
    let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
    assert!(terms.is_empty(), "gallery carries no vocabulary");
}

fn vocab_srsj() -> String {
    serde_json::json!({
        "srsj": "1",
        "manifest": {
            "repositoryId": "test-repo-vocab",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "instanceIndex": [],
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "id": "pkg-vocab-001",
                "namespace": "com.test",
                "name": "test-pkg",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "vocabularies": ["vocabularies/tags.json"]
            },
            "package/vocabularies/tags.json": {
                "namespace": "com.test",
                "name": "tags",
                "version": 1,
                "mode": "open",
                "createdAt": "",
                "terms": [
                    {
                        "id": "term-001",
                        "version": 1,
                        "namespace": "com.test",
                        "key": "category:core"
                    }
                ]
            }
        }
    })
    .to_string()
}

#[test]
fn list_tags_returns_terms_from_vocabulary() {
    let store = JsonStore::from_srsj(&vocab_srsj()).expect("vocab srsj must load");
    let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
    assert_eq!(terms.len(), 1, "one term registered");
    assert_eq!(terms[0].key, "category:core");
    assert_eq!(terms[0].namespace, "com.test");
}
