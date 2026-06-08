/// generate-schemas — writes JSON Schema golden files for every CLI payload type.
///
/// Run this whenever payload structs change:
///   cargo run --bin generate-schemas
///
/// The generated files are committed to `crates/srs-cli/schemas/payload/`.
/// CI runs `cargo test -p srs payload_contracts` which compares live schema output
/// against these committed files; a diff means the contract changed and the developer
/// must regenerate + commit the updated golden files.
fn main() {
    use schemars::schema_for;
    use srs::payload::*;
    use std::path::Path;

    let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/payload");
    std::fs::create_dir_all(&schema_dir).expect("create schema dir");

    macro_rules! write_schema {
        ($name:literal, $T:ty) => {{
            let schema = schema_for!($T);
            let json = serde_json::to_string_pretty(&schema).expect("serialize schema");
            let path = schema_dir.join(concat!($name, ".json"));
            std::fs::write(&path, json + "\n").expect(concat!("write ", $name, ".json"));
            println!("wrote {}", path.display());
        }};
    }

    // Note payloads
    write_schema!("note-list", NoteListPayload);
    write_schema!("note-get", NotePayload);
    write_schema!("note-create", NotePayload);
    write_schema!("note-update", NotePayload);
    write_schema!("note-delete", DeletedPayload);
    write_schema!("note-tag-add", NoteTagAddPayload);
    write_schema!("note-tag-remove", NoteTagRemovePayload);
    write_schema!("note-tag-list", NoteTagListPayload);
    write_schema!("note-tag-map", NoteTagMapPayload);
    write_schema!("note-foundations", NoteFoundationsPayload);

    // Record payloads
    write_schema!("record-list", RecordListPayload);
    write_schema!("record-get", RecordPayload);
    write_schema!("record-create", RecordPayload);
    write_schema!("record-update", RecordPayload);
    write_schema!("record-validate", RecordValidatePayload);
    write_schema!("record-delete", DeletedPayload);
    write_schema!("record-transition", RecordPayload);
    write_schema!("record-successor", RecordSuccessorPayload);
    write_schema!("record-revision-list", RevisionListPayload);
    write_schema!("record-revision-get", RevisionPayload);
    write_schema!("record-tag-add", RecordTagAddPayload);
    write_schema!("record-tag-remove", RecordTagAddPayload);
    write_schema!("record-tag-list", RecordTagListPayload);

    // Relation payloads
    write_schema!("relation-list", RelationListPayload);
    write_schema!("relation-get", RelationPayload);
    write_schema!("relation-create", RelationPayload);
    write_schema!("relation-delete", RelationDeletePayload);

    // Container payloads
    write_schema!("container-list", ContainerListPayload);
    write_schema!("container-get", ContainerPayload);
    write_schema!("container-create", ContainerPayload);
    write_schema!("container-update", ContainerPayload);
    write_schema!("container-delete", ContainerDeletePayload);
    write_schema!("container-members-list", ContainerMembersPayload);
    write_schema!("container-members-add", ContainerMembersMutatePayload);
    write_schema!("container-members-remove", ContainerMembersMutatePayload);
    write_schema!("container-roots-list", ContainerRootsPayload);
    write_schema!("container-roots-add", ContainerRootsMutatePayload);
    write_schema!("container-roots-remove", ContainerRootsMutatePayload);
    write_schema!("container-validate", ContainerValidatePayload);

    // Tag payloads
    write_schema!("tag-list", TagListPayload);
    write_schema!("tag-get", TagPayload);
    write_schema!("tag-create", TagPayload);
    write_schema!("tag-update", TagPayload);
    write_schema!("tag-delete", DeletedPayload);

    // Field payloads
    write_schema!("field-list", FieldListPayload);
    write_schema!("field-get", FieldPayload);
    write_schema!("field-create", FieldPayload);
    write_schema!("field-update", FieldPayload);
    write_schema!("field-delete", FieldDeletePayload);

    // Type payloads
    write_schema!("type-list", TypeListPayload);
    write_schema!("type-get", TypePayload);
    write_schema!("type-create", TypePayload);
    write_schema!("type-update", TypePayload);
    write_schema!("type-delete", TypeDeletePayload);
    write_schema!("type-schema", TypeSchemaPayload);

    // Extension payloads
    write_schema!("extension-list", ExtensionListPayload);
    write_schema!("extension-get", ExtensionPayload);
    write_schema!("extension-create", ExtensionPayload);
    write_schema!("extension-update", ExtensionPayload);
    write_schema!("extension-delete", DeletedPayload);

    // Protocol payloads
    write_schema!("protocol-list", ProtocolListPayload);
    write_schema!("protocol-get", ProtocolPayload);
    write_schema!("protocol-create", ProtocolPayload);
    write_schema!("protocol-stages", ProtocolStagesPayload);
    write_schema!("protocol-validate", ProtocolValidatePayload);
    write_schema!("protocol-update", ProtocolPayload);
    write_schema!("protocol-delete", ProtocolDeletePayload);

    // Blueprint payloads
    write_schema!("blueprint-list", BlueprintListPayload);
    write_schema!("blueprint-get", BlueprintPayload);
    write_schema!("blueprint-create", BlueprintPayload);
    write_schema!("blueprint-update", BlueprintPayload);
    write_schema!("blueprint-delete", BlueprintDeletePayload);
    write_schema!("blueprint-validate", BlueprintValidatePayload);
    write_schema!("blueprint-structure", BlueprintStructurePayload);
    write_schema!("blueprint-schema", BlueprintSchemaPayload);

    // View payloads
    write_schema!("view-list", ViewListPayload);
    write_schema!("view-get", ViewPayload);
    write_schema!("view-create", ViewPayload);
    write_schema!("view-update", ViewPayload);
    write_schema!("view-delete", ViewDeletePayload);

    // Document-view payloads
    write_schema!("document-view-list", DocumentViewListPayload);
    write_schema!("document-view-get", DocumentViewPayload);
    write_schema!("document-view-create", DocumentViewPayload);
    write_schema!("document-view-update", DocumentViewPayload);
    write_schema!("document-view-delete", DocumentViewDeletePayload);

    // Theme payloads
    write_schema!("theme-list", ThemeListPayload);
    write_schema!("theme-get", ThemePayload);
    write_schema!("theme-create", ThemePayload);
    write_schema!("theme-update", ThemePayload);
    write_schema!("theme-delete", ThemeDeletePayload);

    // Render payloads
    write_schema!("render-document-view", RenderDocumentViewPayload);

    // Repo payloads
    write_schema!("repo-create", RepoCreatePayload);
    write_schema!("repo-map", RepoMapPayload);
    write_schema!("repo-copy", RepoCopyPayload);
    write_schema!("repo-validate", RepoValidatePayload);
    write_schema!("repo-extensions-list", RepoExtensionsPayload);
    write_schema!("repo-extensions-enable", RepoExtensionsMutatePayload);
    write_schema!("repo-extensions-disable", RepoExtensionsMutatePayload);

    // Package payloads
    write_schema!("package-list", PackageListPayload);
    write_schema!("package-create", PackageCreatePayload);
    write_schema!("package-import", PackageImportPayload);
    write_schema!("package-update", PackageUpdatePayload);
    write_schema!("package-refs", PackageRefPayload);

    // Vocabulary payloads (RFC-006)
    write_schema!("vocabulary-list", VocabularyListPayload);
    write_schema!("vocabulary-get", VocabularyGetPayload);
    write_schema!("vocabulary-create", VocabularyCreatePayload);
    write_schema!("vocabulary-term-create", TermCreatePayload);

    // Lifecycle payloads (RFC-006)
    write_schema!("lifecycle-list", LifecycleListPayload);
    write_schema!("lifecycle-get", LifecycleGetPayload);

    // Term payloads (RFC-006)
    write_schema!("term-list", TermListPayload);
    write_schema!("term-get", TermGetPayload);

    // Vocabulary promote payload (RFC-006)
    write_schema!("vocabulary-promote", PromoteVocabularyPayload);
    write_schema!(
        "vocabulary-promote-blocked",
        PromoteVocabularyBlockedPayload
    );
    write_schema!("vocabulary-derive-tag-set", VocabularyDeriveTagSetPayload);

    // Tree payloads
    write_schema!("tree", TreePayload);

    println!("done.");
}
