/// Golden-file tests for CLI payload JSON schemas.
///
/// Each test regenerates the JSON Schema for a payload type and compares it
/// against the committed golden file in `schemas/payload/`.
///
/// On failure, run `cargo run --bin generate-schemas` to regenerate the golden
/// files, inspect the diff, and commit the updated files.
///
/// This ensures that any rename, addition, or removal of payload fields is an
/// explicit, reviewed change visible in the PR diff.
use srs::payload::*;
use std::path::Path;

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::schema_for!(T);
    let generated = serde_json::to_string_pretty(&schema).expect("serialize schema") + "\n";

    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/payload")
        .join(format!("{name}.json"));

    if !golden_path.exists() {
        panic!(
            "Golden schema file not found: {}\nRun `cargo run --bin generate-schemas` to create it.",
            golden_path.display()
        );
    }

    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", golden_path.display()));

    if generated != golden {
        panic!(
            "Payload contract changed for '{name}'.\n\
             Run `cargo run --bin generate-schemas` to regenerate golden files, \
             review the diff, and commit the updated schema.\n\
             --- expected (golden) ---\n{golden}\
             --- actual (generated) ---\n{generated}"
        );
    }
}

// ── Note ──────────────────────────────────────────────────────────────────────

#[test]
fn note_list() {
    check::<NoteListPayload>("note-list");
}

#[test]
fn note_get() {
    check::<NotePayload>("note-get");
}

#[test]
fn note_delete() {
    check::<DeletedPayload>("note-delete");
}

#[test]
fn note_tag_add() {
    check::<NoteTagAddPayload>("note-tag-add");
}

#[test]
fn note_tag_remove() {
    check::<NoteTagRemovePayload>("note-tag-remove");
}

#[test]
fn note_tag_list() {
    check::<NoteTagListPayload>("note-tag-list");
}

#[test]
fn note_tag_map() {
    check::<NoteTagMapPayload>("note-tag-map");
}

#[test]
fn note_foundations() {
    check::<NoteFoundationsPayload>("note-foundations");
}

// ── Record ────────────────────────────────────────────────────────────────────

#[test]
fn record_list() {
    check::<RecordListPayload>("record-list");
}

#[test]
fn record_get() {
    check::<RecordPayload>("record-get");
}

#[test]
fn record_delete() {
    check::<DeletedPayload>("record-delete");
}

#[test]
fn record_transition() {
    check::<RecordPayload>("record-transition");
}

#[test]
fn record_successor() {
    check::<RecordSuccessorPayload>("record-successor");
}

#[test]
fn record_revision_list() {
    check::<RevisionListPayload>("record-revision-list");
}

#[test]
fn record_revision_get() {
    check::<RevisionPayload>("record-revision-get");
}

// ── Relation ──────────────────────────────────────────────────────────────────

#[test]
fn relation_list() {
    check::<RelationListPayload>("relation-list");
}

#[test]
fn relation_get() {
    check::<RelationPayload>("relation-get");
}

#[test]
fn relation_delete() {
    check::<RelationDeletePayload>("relation-delete");
}

// ── Container ─────────────────────────────────────────────────────────────────

#[test]
fn container_list() {
    check::<ContainerListPayload>("container-list");
}

#[test]
fn container_get() {
    check::<ContainerPayload>("container-get");
}

#[test]
fn container_delete() {
    check::<ContainerDeletePayload>("container-delete");
}

#[test]
fn container_members_list() {
    check::<ContainerMembersPayload>("container-members-list");
}

#[test]
fn container_members_add() {
    check::<ContainerMembersMutatePayload>("container-members-add");
}

#[test]
fn container_roots_list() {
    check::<ContainerRootsPayload>("container-roots-list");
}

#[test]
fn container_roots_add() {
    check::<ContainerRootsMutatePayload>("container-roots-add");
}

#[test]
fn container_validate() {
    check::<ContainerValidatePayload>("container-validate");
}

// ── Tag ───────────────────────────────────────────────────────────────────────

#[test]
fn tag_list() {
    check::<TagListPayload>("tag-list");
}

#[test]
fn tag_get() {
    check::<TagPayload>("tag-get");
}

#[test]
fn tag_delete() {
    check::<DeletedPayload>("tag-delete");
}

// ── Field ─────────────────────────────────────────────────────────────────────

#[test]
fn field_list() {
    check::<FieldListPayload>("field-list");
}

#[test]
fn field_get() {
    check::<FieldPayload>("field-get");
}

// ── Type ──────────────────────────────────────────────────────────────────────

#[test]
fn type_list() {
    check::<TypeListPayload>("type-list");
}

#[test]
fn type_get() {
    check::<TypePayload>("type-get");
}

// ── Extension ─────────────────────────────────────────────────────────────────

#[test]
fn extension_list() {
    check::<ExtensionListPayload>("extension-list");
}

#[test]
fn extension_get() {
    check::<ExtensionPayload>("extension-get");
}

#[test]
fn extension_delete() {
    check::<DeletedPayload>("extension-delete");
}

// ── Protocol ──────────────────────────────────────────────────────────────────

#[test]
fn protocol_list() {
    check::<ProtocolListPayload>("protocol-list");
}

#[test]
fn protocol_get() {
    check::<ProtocolPayload>("protocol-get");
}

#[test]
fn protocol_stages() {
    check::<ProtocolStagesPayload>("protocol-stages");
}

#[test]
fn protocol_validate() {
    check::<ProtocolValidatePayload>("protocol-validate");
}

#[test]
fn protocol_update() {
    check::<ProtocolPayload>("protocol-update");
}

#[test]
fn protocol_delete() {
    check::<ProtocolDeletePayload>("protocol-delete");
}

// ── View ──────────────────────────────────────────────────────────────────────

#[test]
fn view_list() {
    check::<ViewListPayload>("view-list");
}

#[test]
fn view_get() {
    check::<ViewPayload>("view-get");
}

#[test]
fn view_delete() {
    check::<ViewDeletePayload>("view-delete");
}

// ── Document view ─────────────────────────────────────────────────────────────

#[test]
fn document_view_list() {
    check::<DocumentViewListPayload>("document-view-list");
}

#[test]
fn document_view_get() {
    check::<DocumentViewPayload>("document-view-get");
}

#[test]
fn document_view_delete() {
    check::<DocumentViewDeletePayload>("document-view-delete");
}

// ── Render ────────────────────────────────────────────────────────────────────

#[test]
fn render_document_view() {
    check::<RenderDocumentViewPayload>("render-document-view");
}

// ── Repo ──────────────────────────────────────────────────────────────────────

#[test]
fn repo_create() {
    check::<RepoCreatePayload>("repo-create");
}

#[test]
fn repo_map() {
    check::<RepoMapPayload>("repo-map");
}

#[test]
fn repo_copy() {
    check::<RepoCopyPayload>("repo-copy");
}

#[test]
fn repo_validate() {
    check::<RepoValidatePayload>("repo-validate");
}

#[test]
fn repo_extensions_list() {
    check::<RepoExtensionsPayload>("repo-extensions-list");
}

#[test]
fn repo_extensions_enable() {
    check::<RepoExtensionsMutatePayload>("repo-extensions-enable");
}

// ── Package ───────────────────────────────────────────────────────────────────

#[test]
fn package_list() {
    check::<PackageListPayload>("package-list");
}

#[test]
fn package_create() {
    check::<PackageCreatePayload>("package-create");
}

#[test]
fn package_import() {
    check::<PackageImportPayload>("package-import");
}

#[test]
fn package_update() {
    check::<PackageUpdatePayload>("package-update");
}

#[test]
fn package_refs() {
    check::<PackageRefPayload>("package-refs");
}

// ── Vocabulary (RFC-006) ──────────────────────────────────────────────────────

#[test]
fn vocabulary_list() {
    check::<VocabularyListPayload>("vocabulary-list");
}

#[test]
fn vocabulary_get() {
    check::<VocabularyGetPayload>("vocabulary-get");
}

// ── Lifecycle (RFC-006) ───────────────────────────────────────────────────────

#[test]
fn lifecycle_list() {
    check::<LifecycleListPayload>("lifecycle-list");
}

#[test]
fn lifecycle_get() {
    check::<LifecycleGetPayload>("lifecycle-get");
}
