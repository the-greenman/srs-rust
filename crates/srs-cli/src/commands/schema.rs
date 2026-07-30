use crate::commands::{with_store, CliContext, SchemaCommand};
use crate::output;
use crate::payload::SchemaGeneratePayload;
use anyhow::Result;
use srs_projection::json_schema::to_canonical_json;
use srs_projection::{schema_bundle, SchemaBundleInput};

/// The two frozen meta-model entities, emitted when no `--entity` is given.
const DEFAULT_ENTITIES: &[&str] = &["field", "type"];

pub fn dispatch(ctx: CliContext, cmd: SchemaCommand) -> Result<String> {
    match cmd {
        SchemaCommand::Generate { entities } => cmd_schema_generate(ctx, entities),
    }
}

fn cmd_schema_generate(ctx: CliContext, entities: Vec<String>) -> Result<String> {
    let entities = if entities.is_empty() {
        DEFAULT_ENTITIES.iter().map(|e| e.to_string()).collect()
    } else {
        entities
    };
    match with_store(&ctx, |store| {
        Ok(schema_bundle(store, SchemaBundleInput { entities })?)
    }) {
        Ok(result) => {
            let canonical_json = to_canonical_json(&result.bundle)?;
            output::serialize_with_diagnostics(
                "schema generate",
                SchemaGeneratePayload {
                    bundle: result.bundle,
                    canonical_json,
                },
                result.inexpressible,
            )
        }
        Err(e) => Ok(output::err("schema generate", vec![e.to_string()])),
    }
}
