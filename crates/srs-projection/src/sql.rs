//! Relational (SQL) projection of a repository (srs-rust#1603).
//!
//! The packages define the schema: one table per Type, keyed by `instance_id`,
//! with the effective fields (inheritance flattened) as typed columns; lists and
//! inline composites become owned child tables; the fixed tables carry what is
//! not a Type (instances, notes, tags, relations, containers). With
//! `include_data`, every record, note, relation and container entry follows as
//! `INSERT` statements.
//!
//! This is a derived, rebuildable projection (ADR-041 §5, G8): never a store the
//! files are loaded from. Whatever the model defines and SQL cannot carry is
//! named in `inexpressible`, never silently dropped — the same contract as the
//! RFC-035 JSON Schema projection.
//!
//! The emitted SQL is the common subset SQLite and PostgreSQL both accept.

use serde::Serialize;
use srs_core::types::field::Field;
use srs_core::types::field_type::{AllowedValue, Datatype, FieldType, RefMode};
use srs_core::types::record_type::{FieldAssignment, RecordType};
use srs_repository::container_service::{list_containers, ContainerListFilter};
use srs_repository::error::RepositoryError;
use srs_repository::index::InstanceQuery;
use srs_repository::package::Package;
use srs_repository::store::RepositoryStore;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Input contract for [`sql_projection`].
#[derive(Debug, Clone, Default)]
pub struct SqlProjectionInput {
    /// Emit `INSERT` statements for every record, note, relation and container
    /// entry after the DDL.
    pub include_data: bool,
}

/// One generated table, for consumers that want the map without parsing SQL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlTable {
    /// The unquoted table name.
    pub name: String,
    /// What the table projects: `type`, `list`, `inline` or `fixed`.
    pub kind: String,
    /// The Type the table projects (`type` and `inline` kinds), as `typeId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_version: Option<u32>,
}

/// Output contract for [`sql_projection`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlProjectionResult {
    /// The DDL (and, with `include_data`, the inserts): one statement per line.
    pub sql: String,
    pub tables: Vec<SqlTable>,
    /// Rows emitted, by table kind (`records`, `notes`, `relations`,
    /// `containerEntries`); all zero without `include_data`.
    pub rows: SqlRowCounts,
    /// Constraints the projection could not express, named explicitly.
    pub inexpressible: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlRowCounts {
    pub records: u64,
    pub notes: u64,
    pub relations: u64,
    pub container_entries: u64,
}

/// Project the repository's packages into relational DDL, and optionally its
/// instances into rows.
pub fn sql_projection(
    store: &dyn RepositoryStore,
    input: SqlProjectionInput,
) -> Result<SqlProjectionResult, RepositoryError> {
    let package = store.load_package()?;
    let plan = Plan::build(&package)?;
    let mut out = String::new();
    let mut inexpressible = plan.inexpressible.clone();
    plan.emit_ddl(&mut out);
    let mut rows = SqlRowCounts::default();
    if input.include_data {
        emit_data(store, &plan, &mut out, &mut rows, &mut inexpressible)?;
    }
    inexpressible.sort();
    inexpressible.dedup();
    Ok(SqlProjectionResult {
        sql: out,
        tables: plan.tables(),
        rows,
        inexpressible,
    })
}

// ---------------------------------------------------------------- planning

struct Column {
    field: Field,
    required: bool,
    /// SQL column name (unquoted).
    name: String,
    sql_type: &'static str,
    /// Target table for a `ref` reference field.
    references: Option<String>,
    json: bool,
    checks: Vec<String>,
}

struct ListTable {
    field: Field,
    name: String,
    sql_type: &'static str,
    references: Option<String>,
}

struct Inline {
    field: Field,
    name: String,
    child_key: String,
    list: bool,
}

struct TablePlan {
    table: String,
    type_id: String,
    type_version: u32,
    /// `true` when rows are owned by a parent row (inline composite range).
    owned: bool,
    columns: Vec<Column>,
    lists: Vec<ListTable>,
    inlines: Vec<Inline>,
}

struct Plan {
    /// Keyed by `typeId@version`, in deterministic emission order.
    types: BTreeMap<String, TablePlan>,
    order: Vec<String>,
    inexpressible: Vec<String>,
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    let mut last_us = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

fn table_name(t: &RecordType) -> String {
    let tail = t.namespace.rsplit('.').next().unwrap_or(&t.namespace);
    let base = snake(&format!("{tail}_{}", t.name));
    if t.version > 1 {
        format!("{base}_v{}", t.version)
    } else {
        base
    }
}

fn type_key(id: &str, version: u32) -> String {
    format!("{id}@{version}")
}

fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn scalar_sql_type(dt: Datatype) -> &'static str {
    match dt {
        Datatype::Integer => "INTEGER",
        Datatype::Number => "REAL",
        Datatype::Boolean => "INTEGER",
        _ => "TEXT",
    }
}

impl Plan {
    fn build(package: &Package) -> Result<Self, RepositoryError> {
        let mut sorted: Vec<&RecordType> = package.record_types.iter().collect();
        sorted.sort_by(|a, b| {
            (&a.namespace, &a.name, a.version).cmp(&(&b.namespace, &b.name, b.version))
        });
        let mut plan = Plan {
            types: BTreeMap::new(),
            order: Vec::new(),
            inexpressible: Vec::new(),
        };
        // Inline ranges are owned child tables: find them first so the type
        // table knows whether it is keyed by instance_id or by owner.
        let mut owned = std::collections::HashSet::new();
        for t in &sorted {
            for fa in package.effective_fields(t)? {
                if let Some(f) = package.resolve_field(&fa.field_id) {
                    if f.field_type.datatype == Datatype::Ref
                        && f.field_type.effective_mode() == RefMode::Inline
                    {
                        if let Some(r) = &f.field_type.range_type {
                            owned.insert(type_key(&r.type_id, r.type_version));
                        }
                    }
                }
            }
        }
        for t in sorted {
            let key = type_key(&t.id, t.version);
            let table = table_name(t);
            let mut tp = TablePlan {
                table: table.clone(),
                type_id: t.id.clone(),
                type_version: t.version,
                owned: owned.contains(&key),
                columns: Vec::new(),
                lists: Vec::new(),
                inlines: Vec::new(),
            };
            for fa in package.effective_fields(t)? {
                plan.plan_field(package, t, &mut tp, &fa);
            }
            plan.types.insert(key.clone(), tp);
            plan.order.push(key);
        }
        if plan.types.values().any(|t| t.owned) {
            plan.inexpressible.push(
                "inline composites: owner_table/owner_row is a polymorphic reference, not a \
                 foreign key"
                    .to_string(),
            );
        }
        Ok(plan)
    }

    fn plan_field(
        &mut self,
        package: &Package,
        t: &RecordType,
        tp: &mut TablePlan,
        fa: &FieldAssignment,
    ) {
        let Some(field) = package.resolve_field(&fa.field_id) else {
            self.inexpressible.push(format!(
                "type '{}': fieldId {} does not resolve; no column emitted",
                t.name, fa.field_id
            ));
            return;
        };
        let ft = &field.field_type;
        let name = snake(&field.name);
        let list = ft.is_list();
        match ft.datatype {
            Datatype::Ref => {
                let Some(range) = &ft.range_type else {
                    self.inexpressible.push(format!(
                        "type '{}': ref field '{}' has no rangeType; no column emitted",
                        t.name, field.name
                    ));
                    return;
                };
                let Some(rt) = package.resolve_type(&range.type_id, range.type_version) else {
                    self.inexpressible.push(format!(
                        "type '{}': ref field '{}' ranges over unresolved type {}@{}; no column \
                         emitted",
                        t.name, field.name, range.type_id, range.type_version
                    ));
                    return;
                };
                let target = table_name(rt);
                if ft.effective_mode() == RefMode::Inline {
                    if fa.required {
                        self.inexpressible.push(format!(
                            "type '{}': required inline composite '{}': presence is not a column \
                             constraint",
                            t.name, field.name
                        ));
                    }
                    tp.inlines.push(Inline {
                        field: field.clone(),
                        name,
                        child_key: type_key(&range.type_id, range.type_version),
                        list,
                    });
                } else if list {
                    self.note_required_list(t, field, fa);
                    tp.lists.push(ListTable {
                        field: field.clone(),
                        name: format!("{}__{name}", tp.table),
                        sql_type: "TEXT",
                        references: Some(target),
                    });
                } else {
                    tp.columns.push(Column {
                        field: field.clone(),
                        required: fa.required,
                        name,
                        sql_type: "TEXT",
                        references: Some(target),
                        json: false,
                        checks: Vec::new(),
                    });
                }
            }
            Datatype::Map | Datatype::Dependent => {
                self.inexpressible.push(format!(
                    "type '{}': field '{}' is `{}`; stored as JSON text, unconstrained",
                    t.name,
                    field.name,
                    ft.datatype.as_str()
                ));
                tp.columns.push(Column {
                    field: field.clone(),
                    required: fa.required,
                    name,
                    sql_type: "TEXT",
                    references: None,
                    json: true,
                    checks: Vec::new(),
                });
            }
            dt => {
                if list {
                    self.note_required_list(t, field, fa);
                    tp.lists.push(ListTable {
                        field: field.clone(),
                        name: format!("{}__{name}", tp.table),
                        sql_type: scalar_sql_type(dt),
                        references: None,
                    });
                } else {
                    let checks = column_checks(package, &name, ft, &mut self.inexpressible);
                    tp.columns.push(Column {
                        field: field.clone(),
                        required: fa.required,
                        name,
                        sql_type: scalar_sql_type(dt),
                        references: None,
                        json: false,
                        checks,
                    });
                }
                if let Some(c) = &ft.constraints {
                    if c.pattern.is_some() {
                        self.inexpressible.push(format!(
                            "type '{}': field '{}' has a pattern constraint; portable SQL has no \
                             REGEXP",
                            t.name, field.name
                        ));
                    }
                }
            }
        }
    }

    fn note_required_list(&mut self, t: &RecordType, field: &Field, fa: &FieldAssignment) {
        if fa.required || field.field_type.min_items.unwrap_or(0) > 0 {
            self.inexpressible.push(format!(
                "type '{}': list field '{}' must be non-empty; not a column constraint",
                t.name, field.name
            ));
        }
    }

    fn emit_ddl(&self, out: &mut String) {
        for stmt in FIXED_DDL {
            out.push_str(stmt);
            out.push('\n');
        }
        for key in &self.order {
            let tp = &self.types[key];
            let mut cols: Vec<String> = Vec::new();
            if tp.owned {
                cols.push("row_id INTEGER PRIMARY KEY".into());
                cols.push("owner_table TEXT NOT NULL".into());
                cols.push("owner_row TEXT NOT NULL".into());
                cols.push("owner_field TEXT NOT NULL".into());
                cols.push("seq INTEGER NOT NULL".into());
            } else {
                cols.push("instance_id TEXT PRIMARY KEY REFERENCES instance(instance_id)".into());
            }
            for c in &tp.columns {
                let mut def = format!("{} {}", q(&c.name), c.sql_type);
                if c.required {
                    def.push_str(" NOT NULL");
                }
                if let Some(r) = &c.references {
                    let _ = write!(def, " REFERENCES {}(instance_id)", q(r));
                }
                for chk in &c.checks {
                    def.push(' ');
                    def.push_str(chk);
                }
                cols.push(def);
            }
            let _ = writeln!(out, "CREATE TABLE {} ({});", q(&tp.table), cols.join(", "));
            for l in &tp.lists {
                let owner_ref = if tp.owned {
                    String::new()
                } else {
                    format!(" REFERENCES {}(instance_id)", q(&tp.table))
                };
                let value_ref = l
                    .references
                    .as_ref()
                    .map(|r| format!(" REFERENCES {}(instance_id)", q(r)))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "CREATE TABLE {} (owner_row TEXT NOT NULL{owner_ref}, seq INTEGER NOT NULL, \
                     value {}{value_ref}, PRIMARY KEY (owner_row, seq));",
                    q(&l.name),
                    l.sql_type
                );
            }
        }
    }

    fn tables(&self) -> Vec<SqlTable> {
        let mut v: Vec<SqlTable> = FIXED_TABLES
            .iter()
            .map(|n| SqlTable {
                name: n.to_string(),
                kind: "fixed".into(),
                type_id: None,
                type_version: None,
            })
            .collect();
        for key in &self.order {
            let tp = &self.types[key];
            v.push(SqlTable {
                name: tp.table.clone(),
                kind: if tp.owned { "inline" } else { "type" }.into(),
                type_id: Some(tp.type_id.clone()),
                type_version: Some(tp.type_version),
            });
            for l in &tp.lists {
                v.push(SqlTable {
                    name: l.name.clone(),
                    kind: "list".into(),
                    type_id: Some(tp.type_id.clone()),
                    type_version: Some(tp.type_version),
                });
            }
        }
        v
    }
}

fn column_checks(
    package: &Package,
    col: &str,
    ft: &FieldType,
    out: &mut Vec<String>,
) -> Vec<String> {
    let mut checks = Vec::new();
    let qc = q(col);
    if ft.is_closed() {
        if let Some(allowed) = ft.allowed_values() {
            let vals: Vec<String> = allowed
                .iter()
                .map(|v| match v {
                    AllowedValue::String(s) => lit(s),
                    AllowedValue::Integer(i) => i.to_string(),
                })
                .collect();
            checks.push(format!(
                "CHECK ({qc} IS NULL OR {qc} IN ({}))",
                vals.join(",")
            ));
        } else if let Some(vref) = &ft.vocabulary_ref {
            match package.resolve_vocabulary(vref) {
                Some(v) => {
                    let vals: Vec<String> =
                        v.effective_terms().iter().map(|t| lit(&t.key)).collect();
                    checks.push(format!(
                        "CHECK ({qc} IS NULL OR {qc} IN ({}))",
                        vals.join(",")
                    ));
                }
                None => out.push(format!(
                    "column '{col}': vocabularyRef {vref} does not resolve; no CHECK emitted"
                )),
            }
        }
    }
    if let Some(c) = &ft.constraints {
        if let Some(n) = &c.minimum {
            checks.push(format!("CHECK ({qc} IS NULL OR {qc} >= {n})"));
        }
        if let Some(n) = &c.maximum {
            checks.push(format!("CHECK ({qc} IS NULL OR {qc} <= {n})"));
        }
        if let Some(n) = c.min_length {
            checks.push(format!("CHECK ({qc} IS NULL OR length({qc}) >= {n})"));
        }
        if let Some(n) = c.max_length {
            checks.push(format!("CHECK ({qc} IS NULL OR length({qc}) <= {n})"));
        }
    }
    if ft.datatype == Datatype::Boolean {
        checks.push(format!("CHECK ({qc} IS NULL OR {qc} IN (0,1))"));
    }
    checks
}

const FIXED_TABLES: &[&str] = &[
    "instance",
    "note_section",
    "tag",
    "relation",
    "container",
    "container_member",
];

const FIXED_DDL: &[&str] = &[
    "CREATE TABLE instance (instance_id TEXT PRIMARY KEY, tier INTEGER NOT NULL, type_id TEXT, \
     type_version INTEGER, type_table TEXT, lifecycle_state TEXT, created_at TEXT, updated_at \
     TEXT, created_by_kind TEXT, created_by_id TEXT);",
    "CREATE TABLE note_section (instance_id TEXT NOT NULL REFERENCES instance(instance_id), seq \
     INTEGER NOT NULL, name TEXT NOT NULL, label TEXT, content TEXT NOT NULL, PRIMARY KEY \
     (instance_id, seq));",
    "CREATE TABLE tag (instance_id TEXT NOT NULL REFERENCES instance(instance_id), tag TEXT NOT \
     NULL, PRIMARY KEY (instance_id, tag));",
    "CREATE TABLE relation (relation_id TEXT PRIMARY KEY, relation_type TEXT NOT NULL, \
     source_instance_id TEXT NOT NULL REFERENCES instance(instance_id), target_instance_id TEXT \
     NOT NULL REFERENCES instance(instance_id), created_at TEXT, created_by_kind TEXT, \
     created_by_id TEXT, notes TEXT);",
    "CREATE TABLE container (container_id TEXT PRIMARY KEY, title TEXT NOT NULL, description \
     TEXT, container_type TEXT, anchor_instance_id TEXT, identity_instance_id TEXT, is_root \
     INTEGER NOT NULL DEFAULT 0);",
    "CREATE TABLE container_member (container_id TEXT NOT NULL REFERENCES \
     container(container_id), instance_id TEXT NOT NULL REFERENCES instance(instance_id), seq \
     INTEGER NOT NULL, depth INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (container_id, \
     instance_id));",
];

// ---------------------------------------------------------------- data

fn sql_value(
    v: &serde_json::Value,
    json: bool,
    inexpressible: &mut Vec<String>,
    at: &str,
) -> String {
    if json {
        return lit(&v.to_string());
    }
    match v {
        serde_json::Value::Null => "NULL".into(),
        serde_json::Value::Bool(b) => if *b { "1" } else { "0" }.into(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => lit(s),
        other => {
            inexpressible.push(format!(
                "{at}: structured value in a scalar column, stored as JSON text"
            ));
            lit(&other.to_string())
        }
    }
}

fn actor_cols(actor: Option<&srs_core::types::actor::Actor>) -> (String, String) {
    match actor {
        Some(a) => (
            lit(serde_json::to_value(a.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
                .as_str()),
            lit(&a.id),
        ),
        None => ("NULL".into(), "NULL".into()),
    }
}

fn opt(s: Option<&String>) -> String {
    s.map(|x| lit(x)).unwrap_or_else(|| "NULL".into())
}

struct Owner<'a> {
    table: &'a str,
    row: String,
    field: &'a str,
    seq: usize,
}

/// Emit the rows of one Type table (and its lists and inline children) for
/// one `fieldValues` object. Returns the row key list tables point at.
#[allow(clippy::too_many_arguments)]
fn emit_row(
    plan: &Plan,
    tp: &TablePlan,
    owner: Option<Owner<'_>>,
    instance_id: Option<&str>,
    values: &serde_json::Map<String, serde_json::Value>,
    row_ids: &mut u64,
    out: &mut String,
    inexpressible: &mut Vec<String>,
) {
    let at = |f: &str| format!("table '{}', column '{f}'", tp.table);
    let mut names: Vec<String> = Vec::new();
    let mut vals: Vec<String> = Vec::new();
    let row_key: String;
    if let Some(o) = owner {
        *row_ids += 1;
        row_key = row_ids.to_string();
        names
            .extend(["row_id", "owner_table", "owner_row", "owner_field", "seq"].map(String::from));
        vals.extend([
            row_key.clone(),
            lit(o.table),
            lit(&o.row),
            lit(o.field),
            o.seq.to_string(),
        ]);
    } else {
        row_key = instance_id.unwrap_or_default().to_string();
        names.push("instance_id".into());
        vals.push(lit(&row_key));
    }
    for c in &tp.columns {
        if let Some(v) = values.get(&c.field.name) {
            names.push(q(&c.name));
            vals.push(sql_value(v, c.json, inexpressible, &at(&c.name)));
        }
    }
    let _ = writeln!(
        out,
        "INSERT INTO {} ({}) VALUES ({});",
        q(&tp.table),
        names.join(", "),
        vals.join(", ")
    );
    for l in &tp.lists {
        if let Some(serde_json::Value::Array(items)) = values.get(&l.field.name) {
            for (i, item) in items.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "INSERT INTO {} (owner_row, seq, value) VALUES ({}, {i}, {});",
                    q(&l.name),
                    lit(&row_key),
                    sql_value(item, false, inexpressible, &at(&l.name))
                );
            }
        }
    }
    for inl in &tp.inlines {
        let Some(child) = plan.types.get(&inl.child_key) else {
            continue;
        };
        let Some(v) = values.get(&inl.field.name) else {
            continue;
        };
        let items: Vec<&serde_json::Value> = if inl.list {
            v.as_array().map(|a| a.iter().collect()).unwrap_or_default()
        } else {
            vec![v]
        };
        for (seq, item) in items.into_iter().enumerate() {
            let Some(obj) = item.as_object() else {
                inexpressible.push(format!(
                    "{}: inline composite value is not an object; skipped",
                    at(&inl.name)
                ));
                continue;
            };
            emit_row(
                plan,
                child,
                Some(Owner {
                    table: &tp.table,
                    row: row_key.clone(),
                    field: &inl.name,
                    seq,
                }),
                None,
                obj,
                row_ids,
                out,
                inexpressible,
            );
        }
    }
}

fn emit_data(
    store: &dyn RepositoryStore,
    plan: &Plan,
    out: &mut String,
    rows: &mut SqlRowCounts,
    inexpressible: &mut Vec<String>,
) -> Result<(), RepositoryError> {
    let mut row_ids = 0u64;
    let mut records = srs_repository::record_store::list_all_records(store)?;
    records.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    for r in &records {
        let key = type_key(&r.type_id, r.type_version);
        let Some(tp) = plan.types.get(&key) else {
            inexpressible.push(format!(
                "record {}: type {key} is not in the package; row skipped",
                r.instance_id
            ));
            continue;
        };
        let (ck, ci) = actor_cols(r.created_by.as_ref());
        let _ = writeln!(
            out,
            "INSERT INTO instance (instance_id, tier, type_id, type_version, type_table, \
             lifecycle_state, created_at, updated_at, created_by_kind, created_by_id) VALUES \
             ({}, 2, {}, {}, {}, {}, {}, {}, {ck}, {ci});",
            lit(&r.instance_id),
            lit(&r.type_id),
            r.type_version,
            lit(&tp.table),
            opt(r.lifecycle_state.as_ref()),
            opt(r.created_at.as_ref()),
            opt(r.updated_at.as_ref()),
        );
        emit_tags(&r.instance_id, r.tags.as_deref(), out);
        emit_row(
            plan,
            tp,
            None,
            Some(&r.instance_id),
            &r.field_values.0,
            &mut row_ids,
            out,
            inexpressible,
        );
        rows.records += 1;
    }
    let mut notes = store.list_instances(&InstanceQuery {
        tier: Some(0),
        tag: None,
    })?;
    notes.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    for n in &notes {
        let note = store.load_note_by_id(&n.instance_id)?;
        let (ck, ci) = actor_cols(note.created_by.as_ref());
        let _ = writeln!(
            out,
            "INSERT INTO instance (instance_id, tier, created_at, updated_at, created_by_kind, \
             created_by_id) VALUES ({}, 0, {}, {}, {ck}, {ci});",
            lit(&note.instance_id),
            opt(note.created_at.as_ref()),
            opt(note.updated_at.as_ref()),
        );
        emit_tags(&note.instance_id, note.tags.as_deref(), out);
        if let Some(t) = &note.title {
            let _ = writeln!(
                out,
                "INSERT INTO note_section (instance_id, seq, name, label, content) VALUES ({}, -1, \
                 'title', NULL, {});",
                lit(&note.instance_id),
                lit(t)
            );
        }
        for (i, s) in note.sections.iter().enumerate() {
            let _ = writeln!(
                out,
                "INSERT INTO note_section (instance_id, seq, name, label, content) VALUES ({}, {i}, \
                 {}, {}, {});",
                lit(&note.instance_id),
                lit(&s.name),
                opt(s.label.as_ref()),
                lit(&s.content)
            );
        }
        rows.notes += 1;
    }
    let mut relations = store.list_relations()?;
    relations.sort_by(|a, b| a.relation_id.cmp(&b.relation_id));
    for r in &relations {
        let (ck, ci) = actor_cols(r.created_by.as_ref());
        let _ = writeln!(
            out,
            "INSERT INTO relation (relation_id, relation_type, source_instance_id, \
             target_instance_id, created_at, created_by_kind, created_by_id, notes) VALUES ({}, \
             {}, {}, {}, {}, {ck}, {ci}, {});",
            lit(&r.relation_id),
            lit(&r.relation_type),
            lit(&r.source_instance_id),
            lit(&r.target_instance_id),
            opt(r.created_at.as_ref()),
            opt(r.notes.as_ref()),
        );
        rows.relations += 1;
    }
    let root = store
        .load_manifest()?
        .root_container_id()
        .map(str::to_string);
    let mut summaries = list_containers(store, &ContainerListFilter::default())?;
    summaries.sort_by(|a, b| a.container_id.cmp(&b.container_id));
    for s in &summaries {
        let c = store.load_container(&s.container_id)?;
        let _ = writeln!(
            out,
            "INSERT INTO container (container_id, title, description, container_type, \
             anchor_instance_id, identity_instance_id, is_root) VALUES ({}, {}, {}, {}, {}, {}, \
             {});",
            lit(&c.container_id),
            lit(&c.title),
            opt(c.description.as_ref()),
            opt(c.container_type.as_ref()),
            opt(c.anchor_instance_id.as_ref()),
            opt(c.identity_instance_id.as_ref()),
            u8::from(root.as_deref() == Some(c.container_id.as_str())),
        );
        for (i, e) in c.member_instance_ids.iter().flatten().enumerate() {
            let _ = writeln!(
                out,
                "INSERT INTO container_member (container_id, instance_id, seq, depth) VALUES ({}, \
                 {}, {i}, {});",
                lit(&c.container_id),
                lit(&e.instance_id),
                e.depth.unwrap_or(0)
            );
            rows.container_entries += 1;
        }
    }
    Ok(())
}

fn emit_tags(instance_id: &str, tags: Option<&[String]>, out: &mut String) {
    let mut seen = std::collections::BTreeSet::new();
    for t in tags.unwrap_or_default() {
        if seen.insert(t) {
            let _ = writeln!(
                out,
                "INSERT INTO tag (instance_id, tag) VALUES ({}, {});",
                lit(instance_id),
                lit(t)
            );
        }
    }
}
