use srs_repository::{tag_service, FileStore};

fn gallery_store() -> FileStore {
    let srsj = include_str!("../../srs-repository/tests/fixtures/gallery.srsj");
    srs_repository::srsj::open_srsj(srsj).expect("gallery srsj must load")
}

#[test]
fn list_tags_empty_on_gallery() {
    let store = gallery_store();
    let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
    assert!(terms.is_empty(), "gallery carries no vocabulary");
}

fn vocab_srsj() -> String {
    serde_json::json!({
        "srsj": "2",
        "manifest": {
            "repositoryId": "test-repo-vocab",
            "srsVersion": "2.0-draft",
            "namespace": "com.test",
            "dataModelRevision": 2,
            "packageRef": {"mode": "local", "path": "package"}
        },
        "data": {
            "package/package.json": {
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "pkg-vocab-001",
                "title": "Test Package",
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "namespace": "com.test",
                "name": "test-pkg",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "compositions": [],
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
    let store = srs_repository::srsj::open_srsj(&vocab_srsj()).expect("vocab srsj must load");
    let terms = tag_service::list_terms(&store).expect("list_terms must succeed");
    assert_eq!(terms.len(), 1, "one term registered");
    assert_eq!(terms[0].key, "category:core");
    assert_eq!(terms[0].namespace, "com.test");
}
