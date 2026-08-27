//! SRS projections — the JSON Schema projection (RFC-035 / srs-rust#770), and
//! future SQL, search, and graph views.
//!
//! Per `docs/architecture/capability-layering.md` the semantics live here once,
//! typed in and typed out. The CLI (`srs type json-schema`, `srs schema
//! generate`) and the WASM binding are adapters over these two functions; no
//! client re-derives a schema.

pub mod json_schema;

use json_schema::{
    emit_entity, emit_entity_for, EntitySchema, OrderedMap, ProjectionContext, ProjectionError,
    SchemaBundle,
};
use srs_repository::error::RepositoryError;
use srs_repository::store::RepositoryStore;

/// Input contract for [`type_to_json_schema`].
#[derive(Debug, Clone)]
pub struct TypeToJsonSchemaInput {
    /// The Type to project, by UUID.
    pub type_id: String,
    /// When `None`, the latest version of the Type is projected.
    pub type_version: Option<u32>,
}

/// Output contract for [`type_to_json_schema`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeJsonSchemaResult {
    /// The projected JSON Schema 2020-12 definition schema.
    pub schema: EntitySchema,
    /// Constraints the projection could **not** express in standard JSON
    /// Schema, named explicitly rather than silently dropped. A schema that
    /// looks complete while under-validating is worse than one that says where
    /// it stops.
    pub inexpressible: Vec<String>,
}

/// Input contract for [`schema_bundle`].
#[derive(Debug, Clone)]
pub struct SchemaBundleInput {
    /// Type names to emit, in the order they should appear in the bundle.
    pub entities: Vec<String>,
}

/// Output contract for [`schema_bundle`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBundleResult {
    pub bundle: SchemaBundle,
    pub inexpressible: Vec<String>,
}

/// Project one Type into a standard JSON Schema 2020-12 definition schema.
///
/// This is the executable definition of the standards-compliance claim: a Type
/// projects into a schema that validates its Records, by direct implementation.
/// A model change that breaks the projection becomes a failing test here.
pub fn type_to_json_schema(
    store: &dyn RepositoryStore,
    input: TypeToJsonSchemaInput,
) -> Result<TypeJsonSchemaResult, RepositoryError> {
    let package = store.load_package()?;
    let target = resolve_type(&package, &input.type_id, input.type_version)?;
    let ctx = ProjectionContext::new(&package.namespace, &package.record_types, &package.fields);
    // Project the *resolved* Type, not a re-lookup by name: a name can address
    // several versions (and several namespaces), so re-resolving would quietly
    // return a different Type than the caller asked for.
    let schema = emit_entity_for(&ctx, &target).map_err(to_repository_error)?;
    let inexpressible = collect_inexpressible(&package, &target);
    Ok(TypeJsonSchemaResult {
        schema,
        inexpressible,
    })
}

/// Emit the generated-schema bundle envelope (RFC-035 Change H): the requested
/// entity schemas stamped with the repository's `dataModelRevision`.
pub fn schema_bundle(
    store: &dyn RepositoryStore,
    input: SchemaBundleInput,
) -> Result<SchemaBundleResult, RepositoryError> {
    let package = store.load_package()?;
    let ctx = ProjectionContext::new(&package.namespace, &package.record_types, &package.fields);
    let mut schemas = OrderedMap::new();
    let mut inexpressible = Vec::new();
    for name in &input.entities {
        schemas.insert(
            name.clone(),
            emit_entity(&ctx, name).map_err(to_repository_error)?,
        );
        inexpressible.extend(
            ctx.type_by_name(name)
                .map(|t| collect_inexpressible(&package, t))
                .unwrap_or_default(),
        );
    }
    Ok(SchemaBundleResult {
        bundle: SchemaBundle {
            data_model_revision: srs_repository::field_type_migration_service::data_model_revision(
                store,
            )?,
            schemas,
        },
        inexpressible,
    })
}

/// Resolve a `typeId` (+ optional version) to the Type itself.
///
/// Returning the `RecordType` rather than its name is deliberate: `name` is not
/// a unique key (a UUID lineage has many versions, and two namespaces may share
/// a name), so addressing the projection by name loses the caller's selection.
fn resolve_type(
    package: &srs_repository::package::Package,
    type_id: &str,
    type_version: Option<u32>,
) -> Result<srs_core::types::record_type::RecordType, RepositoryError> {
    let found = match type_version {
        Some(version) => package.resolve_type(type_id, version).cloned(),
        None => package
            .record_types
            .iter()
            .filter(|t| t.id == type_id)
            .max_by_key(|t| t.version)
            .cloned(),
    };
    found.ok_or_else(|| RepositoryError::TypeNotFound {
        type_id: type_id.to_string(),
        version: type_version.unwrap_or(0),
    })
}

/// Name every constraint the projection could not express.
///
/// JSON Schema cannot compare two property *values*, so a `field-ordering`
/// CrossFieldRule stays an engine-side check; and a `dependent` field's
/// conformance to a sibling's type is likewise inexpressible. Both are reported
/// so a consumer knows the schema under-validates rather than assuming it is
/// complete.
fn collect_inexpressible(
    package: &srs_repository::package::Package,
    record_type: &srs_core::types::record_type::RecordType,
) -> Vec<String> {
    use srs_core::types::field::Datatype;
    use srs_core::types::record_type::CrossFieldRuleKind;

    let type_name = &record_type.name;
    let mut out = Vec::new();

    for rule in record_type.validation_rules.iter().flatten() {
        if rule.rule_type == CrossFieldRuleKind::FieldOrdering {
            out.push(format!(
                "type '{type_name}': a field-ordering CrossFieldRule cannot be expressed in JSON \
                 Schema (it compares two property values) — it remains an engine-side check"
            ));
        }
    }

    for assignment in &record_type.fields {
        let Some(field) = package.resolve_field(&assignment.field_id) else {
            continue;
        };
        if field.field_type.datatype == Datatype::Dependent {
            out.push(format!(
                "type '{type_name}': field '{}' is `dependent` on '{}'; JSON Schema cannot express \
                 conformance to another field's type, so the projected node is unconstrained",
                field.name,
                field.field_type.depends_on.as_deref().unwrap_or("self")
            ));
        }
        if let Some(vocabulary_ref) = &field.field_type.vocabulary_ref {
            out.push(format!(
                "type '{type_name}': field '{}' draws from vocabulary '{vocabulary_ref}'; the v1 \
                 projection emits an empty enum rather than a resolved term snapshot",
                field.name
            ));
        }
    }

    out
}

fn to_repository_error(e: ProjectionError) -> RepositoryError {
    RepositoryError::InvalidInput {
        message: e.to_string(),
    }
}
