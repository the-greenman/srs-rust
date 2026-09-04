//! RFC-032 — the decomposed Field type model (`fieldType`).
//!
//! Replaces the pre-RFC-032 scalar `valueType` enum and its untyped companions
//! (`contentFormat`, `allowedValues`, `vocabularyRef`, `validationRules`) with a
//! set of orthogonal facets: **datatype × cardinality × value-domain × format ×
//! constraints**, plus the `ref` / `dependent` / `map` composite datatypes.
//!
//! This module is the Rust counterpart of the spec repo's
//! `scripts/lib/rfc-032-fieldtype.mjs`: [`FieldType::validate`] implements
//! conformance rules R1–R10, and [`LegacyValueType`] + [`FieldType::from_legacy`]
//! implement Change H (the `valueType` → `fieldType` migration). Both are pure —
//! no I/O, no clocks.
//!
//! The wire shape mirrors `$defs/FieldType` in `docs/schema/2.0/field.json`
//! exactly, including property order, so a serialized `FieldType` round-trips
//! byte-stable against the frozen seed.

use serde::{Deserialize, Serialize};

/// RFC-032 Change A — the base datatype facet.
///
/// `ref` = the range is another Type (Change B); `dependent` = the value conforms
/// to another field's type (Change C); `map` = an open string-keyed collection
/// (Change D). Everything else is a portable scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Datatype {
    String,
    Number,
    Integer,
    Boolean,
    Date,
    DateTime,
    Ref,
    Dependent,
    Map,
}

impl Datatype {
    /// The six portable scalars — the datatypes with a direct JSON Schema
    /// `type` (+ `format`) projection and no companion facets.
    pub const fn is_scalar(self) -> bool {
        matches!(
            self,
            Datatype::String
                | Datatype::Number
                | Datatype::Integer
                | Datatype::Boolean
                | Datatype::Date
                | Datatype::DateTime
        )
    }

    /// The wire spelling (`date-time`, not `DateTime`) — used in diagnostics and
    /// by consumers that key on the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Datatype::String => "string",
            Datatype::Number => "number",
            Datatype::Integer => "integer",
            Datatype::Boolean => "boolean",
            Datatype::Date => "date",
            Datatype::DateTime => "date-time",
            Datatype::Ref => "ref",
            Datatype::Dependent => "dependent",
            Datatype::Map => "map",
        }
    }
}

/// RFC-032 R4 — the sole cardinality mechanism. The former `multiselect`
/// valueType and the standalone `repeatable` flag are both subsumed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cardinality {
    #[default]
    Single,
    List,
}

/// RFC-032 R3 — whether a `string` field's value set is open or drawn from a
/// declared vocabulary. `closed` is the successor of `select`/`multiselect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueDomain {
    #[default]
    Open,
    Closed,
}

/// Semantic string format (JSON-Schema-aligned). `date`/`date-time` are
/// first-class datatypes, not formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StringFormat {
    Plain,
    Markdown,
    Uri,
    Uuid,
    Email,
}

/// RFC-032 R8 — how a `ref` field carries its range: nested value(s) conforming
/// to `rangeType`, or the target instance id(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefMode {
    #[default]
    Inline,
    Reference,
}

/// RFC-032 R9 — the value datatype of a `map`, or `open` for a true extension
/// bag. `Ref` (srs#534) is the one composite value range: the map's values are
/// nested objects (or id-shapes, per `mode`) conforming to `rangeType` — the
/// same range-resolution contract `datatype: ref` uses at the top level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MapValueRange {
    String,
    Number,
    Integer,
    Boolean,
    Date,
    DateTime,
    Open,
    Ref,
}

/// RFC-032 R3 / srs#534 — one entry of an inline closed vocabulary.
///
/// A `string` field's `allowedValues` are strings; the "untyped integer enum"
/// capability (`valueDomain: closed` on `datatype: integer`) admits bare
/// integers instead. `#[serde(untagged)]` matches the wire shape exactly: a
/// JSON array mixing string and number literals, never a `{"kind": ...}`
/// wrapper (structured over serialised — no field mixes both in practice, but
/// nothing here enforces that; R3 doesn't either, matching
/// `rfc-032-fieldtype.mjs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedValue {
    String(String),
    Integer(i64),
}

impl From<String> for AllowedValue {
    fn from(s: String) -> Self {
        AllowedValue::String(s)
    }
}

impl From<&str> for AllowedValue {
    fn from(s: &str) -> Self {
        AllowedValue::String(s.to_string())
    }
}

impl From<i64> for AllowedValue {
    fn from(n: i64) -> Self {
        AllowedValue::Integer(n)
    }
}

/// RFC-009 I-78 — a version-exact reference to a Type in the Package. Both
/// members are required; distinct from the pre-RFC-009 spec-level `TypeRef`
/// (Protocol context) where `typeVersion` was optional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactTypeRef {
    pub type_id: String,
    pub type_version: u32,
}

/// RFC-032 R10 / Change F — datatype-appropriate value constraints. Carries the
/// facets the retired `validationRules[]` array used to hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldTypeConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `serde_json::Number`, not `f64`: a Field file writing `"minimum": 1`
    /// must round-trip as `1`, not `1.0`. The RFC-035 projection is held to
    /// byte-parity with the reference emitter, so integer-ness is part of the
    /// wire contract, not a formatting detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<serde_json::Number>,
}

impl FieldTypeConstraints {
    pub fn is_empty(&self) -> bool {
        self.min_length.is_none()
            && self.max_length.is_none()
            && self.pattern.is_none()
            && self.minimum.is_none()
            && self.maximum.is_none()
    }
}

/// RFC-032 Change A — the decomposed value type of a Field.
///
/// Property order matches `$defs/FieldType` in the frozen seed so serialization
/// is byte-stable against it. Optional facets are omitted when absent — a
/// `cardinality: single` field serializes with no `cardinality` key, matching
/// how the spec repo's own migrated Fields are written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldType {
    pub datatype: Datatype,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<Cardinality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_domain: Option<ValueDomain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<AllowedValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<StringFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<FieldTypeConstraints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_type: Option<ExactTypeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RefMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_range: Option<MapValueRange>,
}

// ---------------------------------------------------------------------------
// Constructors — the shapes that appear over and over in real packages.
// ---------------------------------------------------------------------------

impl FieldType {
    /// A bare `fieldType` with every optional facet absent.
    pub fn new(datatype: Datatype) -> Self {
        FieldType {
            datatype,
            cardinality: None,
            min_items: None,
            max_items: None,
            value_domain: None,
            allowed_values: None,
            vocabulary_ref: None,
            format: None,
            constraints: None,
            range_type: None,
            mode: None,
            depends_on: None,
            value_range: None,
        }
    }

    /// `{ datatype: string }` — open, single, unformatted prose.
    pub fn string() -> Self {
        Self::new(Datatype::String)
    }

    /// `{ datatype: string, format: markdown }` — the successor of the
    /// pre-RFC-032 `text` + `contentFormat: markdown` pair.
    pub fn markdown() -> Self {
        Self::new(Datatype::String).with_format(StringFormat::Markdown)
    }

    /// `{ datatype: string, format: plain }` — the successor of the
    /// pre-RFC-032 `text` valueType: multi-line prose with no markup.
    pub fn text() -> Self {
        Self::new(Datatype::String).with_format(StringFormat::Plain)
    }

    /// `{ datatype: string, format: uri }` — the successor of `valueType: url`.
    pub fn uri() -> Self {
        Self::new(Datatype::String).with_format(StringFormat::Uri)
    }

    pub fn number() -> Self {
        Self::new(Datatype::Number)
    }

    pub fn integer() -> Self {
        Self::new(Datatype::Integer)
    }

    pub fn boolean() -> Self {
        Self::new(Datatype::Boolean)
    }

    pub fn date() -> Self {
        Self::new(Datatype::Date)
    }

    pub fn date_time() -> Self {
        Self::new(Datatype::DateTime)
    }

    /// A closed-domain single string over an inline vocabulary — the successor
    /// of `valueType: select` + `allowedValues`.
    pub fn select<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut ft = Self::new(Datatype::String);
        ft.value_domain = Some(ValueDomain::Closed);
        ft.allowed_values = Some(
            values
                .into_iter()
                .map(|v| AllowedValue::String(v.into()))
                .collect(),
        );
        ft
    }

    /// A closed-domain list of strings — the successor of `valueType: multiselect`.
    pub fn multiselect<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut ft = Self::select(values);
        ft.cardinality = Some(Cardinality::List);
        ft
    }

    /// A closed-domain single integer over an inline enum — RFC-032/srs#534's
    /// "untyped integer enum" capability. Projects to a bare `{enum:[...]}`
    /// with no `type` keyword, deliberately distinct from [`FieldType::select`]'s
    /// `{type:"string", enum:[...]}` shape.
    pub fn closed_integer<I>(values: I) -> Self
    where
        I: IntoIterator<Item = i64>,
    {
        let mut ft = Self::new(Datatype::Integer);
        ft.value_domain = Some(ValueDomain::Closed);
        ft.allowed_values = Some(values.into_iter().map(AllowedValue::Integer).collect());
        ft
    }

    /// A closed-domain single string drawing from a named Vocabulary
    /// (`namespace/name@version`) rather than an inline list.
    pub fn closed_by_ref(vocabulary_ref: impl Into<String>) -> Self {
        let mut ft = Self::new(Datatype::String);
        ft.value_domain = Some(ValueDomain::Closed);
        ft.vocabulary_ref = Some(vocabulary_ref.into());
        ft
    }

    /// A closed-domain list drawing from a named Vocabulary.
    pub fn closed_list_by_ref(vocabulary_ref: impl Into<String>) -> Self {
        let mut ft = Self::closed_by_ref(vocabulary_ref);
        ft.cardinality = Some(Cardinality::List);
        ft
    }

    /// A closed-domain string with **no** source set declared — R3-violating by
    /// construction. Exists so tests can build the exact defect
    /// `validate_field_v3` reports; production code should never produce one.
    pub fn closed() -> Self {
        let mut ft = Self::new(Datatype::String);
        ft.value_domain = Some(ValueDomain::Closed);
        ft
    }

    /// The list form of [`FieldType::closed`] — likewise R3-violating.
    pub fn closed_list() -> Self {
        Self::closed().into_list()
    }

    /// An inline `ref` to another Type — a nested object conforming to `range`.
    pub fn inline_ref(range: ExactTypeRef) -> Self {
        let mut ft = Self::new(Datatype::Ref);
        ft.range_type = Some(range);
        ft.mode = Some(RefMode::Inline);
        ft
    }

    /// A `ref` carrying the target instance id rather than a nested object.
    pub fn instance_ref(range: ExactTypeRef) -> Self {
        let mut ft = Self::new(Datatype::Ref);
        ft.range_type = Some(range);
        ft.mode = Some(RefMode::Reference);
        ft
    }

    /// A `map` whose values are inline objects conforming to `rangeType` —
    /// srs#534's "map-of-`$ref`" capability. Same range contract as
    /// [`FieldType::inline_ref`], keyed by string rather than singular.
    pub fn map_of_ref(range: ExactTypeRef) -> Self {
        let mut ft = Self::new(Datatype::Map);
        ft.value_range = Some(MapValueRange::Ref);
        ft.range_type = Some(range);
        ft.mode = Some(RefMode::Inline);
        ft
    }

    /// A `map` whose values are target instance ids rather than nested
    /// objects — the `mode: reference` twin of [`FieldType::map_of_ref`].
    pub fn map_of_ref_ids(range: ExactTypeRef) -> Self {
        let mut ft = Self::new(Datatype::Map);
        ft.value_range = Some(MapValueRange::Ref);
        ft.range_type = Some(range);
        ft.mode = Some(RefMode::Reference);
        ft
    }

    pub fn with_format(mut self, format: StringFormat) -> Self {
        self.format = Some(format);
        self
    }

    pub fn with_cardinality(mut self, cardinality: Cardinality) -> Self {
        self.cardinality = Some(cardinality);
        self
    }

    /// `{ ..., cardinality: list }`.
    pub fn into_list(self) -> Self {
        self.with_cardinality(Cardinality::List)
    }

    pub fn with_constraints(mut self, constraints: FieldTypeConstraints) -> Self {
        self.constraints = Some(constraints);
        self
    }
}

// ---------------------------------------------------------------------------
// Derived facets — the questions consumers actually ask.
// ---------------------------------------------------------------------------

impl FieldType {
    /// Cardinality with R4's default applied (absent ⇒ `single`).
    pub fn effective_cardinality(&self) -> Cardinality {
        self.cardinality.unwrap_or_default()
    }

    /// Value domain with R3's default applied (absent ⇒ `open`).
    pub fn effective_value_domain(&self) -> ValueDomain {
        self.value_domain.unwrap_or_default()
    }

    /// Ref mode with R8's default applied (absent ⇒ `inline`).
    pub fn effective_mode(&self) -> RefMode {
        self.mode.unwrap_or_default()
    }

    pub fn is_list(&self) -> bool {
        self.effective_cardinality() == Cardinality::List
    }

    /// True when the value set is drawn from a declared vocabulary (inline
    /// `allowedValues` or a `vocabularyRef`).
    pub fn is_closed(&self) -> bool {
        self.effective_value_domain() == ValueDomain::Closed
    }

    /// RFC-032 Revision 7 — `effective-single`, the shared cardinality
    /// precondition of I-94, `[T-9]` and `[N+1]`.
    ///
    /// A field is effective-single when `fieldType.cardinality` is absent or
    /// `single`. The transition-only `FieldAssignment.repeatable` conjunct was
    /// removed in the srs#242 Phase-B train (RFC-039 [R7] deletes the trio;
    /// RFC-032 Change-I condition 4), so `cardinality` is the sole mechanism.
    pub fn is_effective_single(&self) -> bool {
        self.effective_cardinality() == Cardinality::Single
    }

    /// The prose `format` allow-list — `format` absent, `plain` or `markdown`.
    ///
    /// Shared by `[T-9]` and `[N+1]`. Enumerated as positives on purpose: an
    /// unrecognised or future `format` is ineligible by default, so no `format`
    /// semantics table has to exist for `uuid` and `email` to be excluded.
    fn has_prose_format(&self) -> bool {
        matches!(
            self.format,
            None | Some(StringFormat::Plain) | Some(StringFormat::Markdown)
        )
    }

    /// I-120 / RFC-012 `[R8]` — whether values of this field are free text a
    /// reader can search.
    ///
    /// `datatype == string` **and** `format` ∈ {absent, `plain`, `markdown`,
    /// `uri`}. Both conjuncts are load-bearing: `format` is what excludes the
    /// string-datatyped but non-prose `uuid` and `email`.
    ///
    /// The `format` clause is an **allow-list**, never a deny-list. Adding a
    /// format to `StringFormat` must not silently make it searchable — a new
    /// format is ineligible until it is enumerated here deliberately.
    ///
    /// A datatype-only test is **not** sufficient, though it looks it. Under the
    /// pre-RFC-032 model the searchable set was
    /// `string | text | url | select | multiselect`, and all five decompose to
    /// `datatype: string` while none of the non-searchable four do — so
    /// `datatype == string` reproduces the legacy eight exactly. That argument
    /// is sound over the legacy eight and unsound over the model as a whole:
    /// RFC-032 admits string-datatyped fields the legacy enum could not express,
    /// and `uuid`/`email` are precisely those. RFC-032 Revision 7 settled the
    /// allow-list above; `rfc012_searchable_set_survives_the_rfc032_decomposition`
    /// still pins the legacy parity that the datatype-only reading was derived
    /// from, and passes under both readings — which is why it cannot be the only
    /// test here.
    ///
    /// The composite datatypes (`ref`, `dependent`, `map`) project structure,
    /// not text, and are excluded outright. No composite recursion is defined by
    /// the erratum: a `ref` whose range has searchable leaves is still not
    /// searchable. That consequence is booked, not overlooked.
    pub fn is_text_searchable(&self) -> bool {
        self.datatype == Datatype::String
            && matches!(
                self.format,
                None | Some(StringFormat::Plain)
                    | Some(StringFormat::Markdown)
                    | Some(StringFormat::Uri)
            )
    }

    /// I-94 / RFC-019 `[R6]` — whether this field may be the `predicateFieldId`
    /// of a `conditional-required` CrossFieldRule.
    ///
    /// Effective-single **and** `datatype` ∈ {`string`, `date`, `date-time`}.
    /// The rule compares a single stored value against a single declared
    /// `predicateValue`, so a list cardinality has no defined semantics.
    pub fn is_conditional_required_eligible(&self) -> bool {
        self.is_effective_single()
            && matches!(
                self.datatype,
                Datatype::String | Datatype::Date | Datatype::DateTime
            )
    }

    /// `[T-9]` / ext:themes-l1 — whether this field may contribute a CSS class
    /// via `Theme.cssClassFields`.
    ///
    /// Effective-single, `datatype == string`, and a prose `format` (absent,
    /// `plain` or `markdown`). `valueDomain` is deliberately **unconstrained**:
    /// both open and closed domains are eligible, since a closed vocabulary is
    /// if anything the more natural source of a stable class name.
    pub fn is_theme_css_class_eligible(&self) -> bool {
        self.is_effective_single() && self.datatype == Datatype::String && self.has_prose_format()
    }

    /// `[N+1]` / ext:views-l2 — whether this field may be a
    /// `DocumentSection.titleFieldId`.
    ///
    /// Effective-single, `datatype == string`, `valueDomain` ∈ {absent, `open`},
    /// and a prose `format`. Stricter than `[T-9]` in exactly one respect: a
    /// closed vocabulary is **not** an eligible heading source.
    pub fn is_title_field_eligible(&self) -> bool {
        self.is_effective_single()
            && self.datatype == Datatype::String
            && self.effective_value_domain() == ValueDomain::Open
            && self.has_prose_format()
    }

    /// Whether values of this field are orderable scalars — the precondition
    /// for a `field-ordering` CrossFieldRule.
    pub fn is_orderable(&self) -> bool {
        matches!(
            self.datatype,
            Datatype::Number | Datatype::Integer | Datatype::Date | Datatype::DateTime
        )
    }

    /// The inline closed vocabulary, when this field declares one.
    pub fn allowed_values(&self) -> Option<&[AllowedValue]> {
        self.allowed_values.as_deref()
    }

    /// A compact, human-readable label — e.g. `string`, `string (markdown)[]`,
    /// `string (closed)`, `ref inline`.
    ///
    /// For prose surfaces (briefs, tables, diagnostics) only. It is lossy by
    /// design: anything that needs the facets must read them, not parse this.
    pub fn describe(&self) -> String {
        let mut s = self.datatype.as_str().to_string();
        let mut qualifiers: Vec<&str> = Vec::new();
        if let Some(format) = self.format {
            qualifiers.push(match format {
                StringFormat::Plain => "plain",
                StringFormat::Markdown => "markdown",
                StringFormat::Uri => "uri",
                StringFormat::Uuid => "uuid",
                StringFormat::Email => "email",
            });
        }
        if self.is_closed() {
            qualifiers.push("closed");
        }
        if self.datatype == Datatype::Ref {
            qualifiers.push(match self.effective_mode() {
                RefMode::Inline => "inline",
                RefMode::Reference => "reference",
            });
        }
        if !qualifiers.is_empty() {
            s.push_str(&format!(" ({})", qualifiers.join(", ")));
        }
        if self.is_list() {
            s.push_str("[]");
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Conformance rules R1–R10 (the Rust twin of `validateFieldType`).
// ---------------------------------------------------------------------------

/// One conformance-rule violation on a `fieldType`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldTypeViolation {
    /// The RFC-032 conformance rule id, e.g. `"R3"`.
    pub rule: &'static str,
    pub message: String,
}

impl std::fmt::Display for FieldTypeViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)
    }
}

impl FieldType {
    /// RFC-032 conformance rules R1–R10 over a single `fieldType`.
    ///
    /// These are the semantic checks JSON Schema cannot express portably (the
    /// seed approximates a few of them with `allOf`/`if`/`then`); this is the
    /// load-bearing correctness check. Mirrors
    /// `scripts/lib/rfc-032-fieldtype.mjs::validateFieldType` rule-for-rule.
    pub fn validate(&self) -> Vec<FieldTypeViolation> {
        let mut v = Vec::new();
        let mut push = |rule: &'static str, message: String| {
            v.push(FieldTypeViolation { rule, message });
        };

        // R2 — `ref` (or `map` with valueRange == ref, srs#534) requires
        // rangeType; rangeType/mode forbidden otherwise.
        let uses_range_type = self.datatype == Datatype::Ref
            || (self.datatype == Datatype::Map && self.value_range == Some(MapValueRange::Ref));
        if uses_range_type {
            if self.range_type.is_none() {
                push(
                    "R2",
                    "datatype ref (or map with valueRange == ref) requires rangeType as a valid \
                     ExactTypeRef"
                        .to_string(),
                );
            } else if let Some(rt) = &self.range_type {
                if rt.type_version < 1 {
                    push("R2", "rangeType.typeVersion must be >= 1".to_string());
                }
                if rt.type_id.is_empty() {
                    push("R2", "rangeType.typeId must not be empty".to_string());
                }
            }
        } else {
            if self.range_type.is_some() {
                push(
                    "R2",
                    "rangeType is only permitted when datatype == ref, or datatype == map with \
                     valueRange == ref"
                        .to_string(),
                );
            }
            if self.mode.is_some() {
                push(
                    "R2",
                    "mode is only permitted when datatype == ref, or datatype == map with \
                     valueRange == ref"
                        .to_string(),
                );
            }
        }

        // R3 — valueDomain only for string/integer (srs#534 adds integer);
        // closed ⇒ exactly one source set.
        match self.value_domain {
            Some(domain) => {
                if !matches!(self.datatype, Datatype::String | Datatype::Integer) {
                    push(
                        "R3",
                        "valueDomain is meaningful only for datatype == string or integer"
                            .to_string(),
                    );
                }
                if domain == ValueDomain::Closed {
                    let has_inline = self.allowed_values.as_ref().is_some_and(|a| !a.is_empty());
                    let has_ref = self.vocabulary_ref.as_ref().is_some_and(|s| !s.is_empty());
                    if has_inline == has_ref {
                        push(
                            "R3",
                            "valueDomain closed requires exactly one of allowedValues or vocabularyRef"
                                .to_string(),
                        );
                    }
                }
            }
            None => {
                if self.allowed_values.is_some() || self.vocabulary_ref.is_some() {
                    push(
                        "R3",
                        "allowedValues/vocabularyRef require valueDomain == closed".to_string(),
                    );
                }
            }
        }

        // R4 — minItems/maxItems only on a list, and 0 <= minItems <= maxItems.
        let is_list = self.is_list();
        if !is_list {
            if self.min_items.is_some() {
                push(
                    "R4",
                    "minItems is only permitted when cardinality == list".to_string(),
                );
            }
            if self.max_items.is_some() {
                push(
                    "R4",
                    "maxItems is only permitted when cardinality == list".to_string(),
                );
            }
        }
        if let (Some(min), Some(max)) = (self.min_items, self.max_items) {
            if min > max {
                push("R4", "minItems must be <= maxItems".to_string());
            }
        }

        // R6 — `dependent` requires dependsOn; dependsOn forbidden otherwise.
        if self.datatype == Datatype::Dependent {
            if self.depends_on.as_ref().is_none_or(|s| s.is_empty()) {
                push(
                    "R6",
                    "datatype dependent requires dependsOn (\"self\" or a sibling field name)"
                        .to_string(),
                );
            }
        } else if self.depends_on.is_some() {
            push(
                "R6",
                "dependsOn is only permitted when datatype == dependent".to_string(),
            );
        }

        // R9 — `map` requires a valueRange; valueRange forbidden otherwise.
        if self.datatype == Datatype::Map {
            if self.value_range.is_none() {
                push(
                    "R9",
                    "datatype map requires valueRange (a scalar datatype or \"open\")".to_string(),
                );
            }
        } else if self.value_range.is_some() {
            push(
                "R9",
                "valueRange is only permitted when datatype == map".to_string(),
            );
        }

        // R10 — constraints must be datatype-appropriate.
        if let Some(c) = &self.constraints {
            let string_constraint =
                c.min_length.is_some() || c.max_length.is_some() || c.pattern.is_some();
            if string_constraint && self.datatype != Datatype::String {
                push(
                    "R10",
                    "constraints minLength/maxLength/pattern apply only to datatype == string"
                        .to_string(),
                );
            }
            let numeric_constraint = c.minimum.is_some() || c.maximum.is_some();
            if numeric_constraint && !matches!(self.datatype, Datatype::Number | Datatype::Integer)
            {
                push(
                    "R10",
                    "constraints minimum/maximum apply only to datatype == number/integer"
                        .to_string(),
                );
            }
        }

        // R5 — `format` is a string-only facet.
        if self.format.is_some() && self.datatype != Datatype::String {
            push(
                "R5",
                "format is meaningful only for datatype == string".to_string(),
            );
        }

        v
    }
}

// ---------------------------------------------------------------------------
// Change H — the `valueType` → `fieldType` migration (data-model revision 0 → 1).
// ---------------------------------------------------------------------------

/// The pre-RFC-032 scalar `valueType` enum.
///
/// Retained **only** so a data-model-revision-0 repository can be read and
/// upgraded ([`FieldType::from_legacy`]). Nothing in the engine may branch on
/// this: the conflation it encodes (datatype × cardinality × value-domain ×
/// format in one enum) is precisely what RFC-032 decomposed. Ask the
/// [`FieldType`] facets instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyValueType {
    String,
    Text,
    Number,
    Boolean,
    Date,
    Url,
    Select,
    Multiselect,
}

/// The pre-RFC-032 `contentFormat` companion, absorbed into
/// [`FieldType::format`] by the migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyContentFormat {
    Plain,
    Markdown,
}

/// The pre-RFC-032 `validationRules[]` entries a Field could carry, absorbed
/// into [`FieldType::constraints`] / [`FieldType::value_domain`] (Change F).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyValidationRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// The companion properties Change H folds into `fieldType` and removes from the
/// Field. Kept as one struct so the migration's inputs are explicit.
#[derive(Debug, Clone, Default)]
pub struct LegacyFieldFacets {
    pub content_format: Option<LegacyContentFormat>,
    pub allowed_values: Option<Vec<String>>,
    pub vocabulary_ref: Option<String>,
    pub validation_rules: Option<Vec<LegacyValidationRule>>,
}

impl FieldType {
    /// RFC-032 Change H — migrate a data-model-revision-0 Field's `valueType`
    /// (plus its companions) into a `fieldType`.
    ///
    /// Total over all eight legacy value types and deterministic: no clocks, no
    /// ordering ambiguity. This is the exact twin of
    /// `scripts/lib/rfc-032-fieldtype.mjs::migrateFieldType`, and the two are
    /// held to the same fixtures.
    pub fn from_legacy(value_type: LegacyValueType, facets: &LegacyFieldFacets) -> Self {
        let markdown = facets.content_format == Some(LegacyContentFormat::Markdown);
        let mut ft = match value_type {
            LegacyValueType::String => {
                let mut ft = FieldType::new(Datatype::String);
                if markdown {
                    ft.format = Some(StringFormat::Markdown);
                }
                ft
            }
            // Multi-line prose collapses to `string`; the plain/markdown
            // distinction moves onto `format` (Change H).
            LegacyValueType::Text => {
                let mut ft = FieldType::new(Datatype::String);
                ft.format = Some(if markdown {
                    StringFormat::Markdown
                } else {
                    StringFormat::Plain
                });
                ft
            }
            LegacyValueType::Number => FieldType::new(Datatype::Number),
            LegacyValueType::Boolean => FieldType::new(Datatype::Boolean),
            LegacyValueType::Date => FieldType::new(Datatype::Date),
            LegacyValueType::Url => {
                let mut ft = FieldType::new(Datatype::String);
                ft.format = Some(StringFormat::Uri);
                ft
            }
            LegacyValueType::Select => {
                let mut ft = FieldType::new(Datatype::String);
                ft.value_domain = Some(ValueDomain::Closed);
                ft
            }
            LegacyValueType::Multiselect => {
                let mut ft = FieldType::new(Datatype::String);
                ft.cardinality = Some(Cardinality::List);
                ft.value_domain = Some(ValueDomain::Closed);
                ft
            }
        };

        // A closed domain draws from exactly one source set (Change A / R3).
        if ft.value_domain == Some(ValueDomain::Closed) {
            match (&facets.allowed_values, &facets.vocabulary_ref) {
                (Some(values), _) if !values.is_empty() => {
                    // Legacy (pre-RFC-032) allowedValues were string-only.
                    ft.allowed_values =
                        Some(values.iter().cloned().map(AllowedValue::String).collect());
                }
                (_, Some(reference)) if !reference.is_empty() => {
                    ft.vocabulary_ref = Some(reference.clone());
                }
                // A closed select with neither is a pre-existing data defect;
                // `validate()` reports it rather than the migration inventing one.
                _ => {}
            }
        }

        // validationRules[] → constraints / valueDomain (Change F).
        for rule in facets.validation_rules.iter().flatten() {
            apply_legacy_validation_rule(&mut ft, rule);
        }

        ft
    }
}

fn apply_legacy_validation_rule(ft: &mut FieldType, rule: &LegacyValidationRule) {
    let as_u32 = |v: &Option<serde_json::Value>| -> Option<u32> {
        match v.as_ref()? {
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    };
    match rule.rule_type.as_str() {
        "minLength" => {
            if let Some(n) = as_u32(&rule.value) {
                ft.constraints
                    .get_or_insert_with(Default::default)
                    .min_length = Some(n);
            }
        }
        "maxLength" => {
            if let Some(n) = as_u32(&rule.value) {
                ft.constraints
                    .get_or_insert_with(Default::default)
                    .max_length = Some(n);
            }
        }
        "pattern" => {
            // `String(rule.value)` in the reference — a non-string pattern is a
            // data defect, but dropping it during a *durable* migration loses
            // the author's intent silently, which is worse than carrying it
            // forward for the conformance check to flag.
            match rule.value.as_ref() {
                Some(serde_json::Value::String(s)) => {
                    ft.constraints.get_or_insert_with(Default::default).pattern = Some(s.clone());
                }
                Some(other) => {
                    ft.constraints.get_or_insert_with(Default::default).pattern =
                        Some(other.to_string());
                }
                None => {}
            }
        }
        "enum" => {
            ft.value_domain = Some(ValueDomain::Closed);
            if let Some(serde_json::Value::Array(items)) = rule.value.as_ref() {
                // Legacy (pre-RFC-032) validationRules enum values were
                // always strings.
                ft.allowed_values = Some(
                    items
                        .iter()
                        .map(|i| match i {
                            serde_json::Value::String(s) => AllowedValue::String(s.clone()),
                            other => AllowedValue::String(other.to_string()),
                        })
                        .collect(),
                );
            }
        }
        // `required` belongs to the FieldAssignment, not the fieldType.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_type_serializes_in_seed_property_order() {
        let ft = FieldType::multiselect(["a", "b"]);
        let s = serde_json::to_string(&ft).unwrap();
        assert_eq!(
            s,
            r#"{"datatype":"string","cardinality":"list","valueDomain":"closed","allowedValues":["a","b"]}"#
        );
    }

    #[test]
    fn absent_optional_facets_are_omitted() {
        let s = serde_json::to_string(&FieldType::string()).unwrap();
        assert_eq!(s, r#"{"datatype":"string"}"#);
    }

    #[test]
    fn date_time_uses_the_hyphenated_wire_spelling() {
        let s = serde_json::to_value(Datatype::DateTime).unwrap();
        assert_eq!(s, json!("date-time"));
        assert_eq!(Datatype::DateTime.as_str(), "date-time");
    }

    #[test]
    fn unknown_field_type_facet_is_rejected() {
        // `deny_unknown_fields` mirrors `additionalProperties: false` on
        // `$defs/FieldType` — the two must not disagree (srs-rust#767).
        let err = serde_json::from_value::<FieldType>(json!({
            "datatype": "string",
            "notAFacet": 1
        }));
        assert!(err.is_err(), "unknown fieldType facet must be rejected");
    }

    #[test]
    fn unknown_keys_in_nested_value_objects_are_rejected() {
        // `$defs/FieldType.properties.constraints` and `$defs/ExactTypeRef` both
        // set `additionalProperties: false`. Accepting an unknown key here and
        // then dropping it on write is silent data loss that a
        // `deny_unknown_fields` on the *outer* Field does not prevent.
        assert!(serde_json::from_value::<FieldType>(json!({
            "datatype": "string",
            "constraints": {"minLength": 1, "bogus": 99}
        }))
        .is_err());
        assert!(serde_json::from_value::<FieldType>(json!({
            "datatype": "ref",
            "rangeType": {"typeId": "T", "typeVersion": 1, "junk": 7}
        }))
        .is_err());
    }

    #[test]
    fn roundtrips_every_datatype() {
        for dt in [
            Datatype::String,
            Datatype::Number,
            Datatype::Integer,
            Datatype::Boolean,
            Datatype::Date,
            Datatype::DateTime,
        ] {
            let ft = FieldType::new(dt);
            let back: FieldType =
                serde_json::from_str(&serde_json::to_string(&ft).unwrap()).unwrap();
            assert_eq!(back, ft);
        }
    }

    // --- derived facets -------------------------------------------------

    #[test]
    fn text_searchability_matches_the_pre_rfc032_set() {
        // string | text | url | select | multiselect were searchable; all five
        // migrate to datatype string. number | boolean | date were not.
        for legacy in [
            LegacyValueType::String,
            LegacyValueType::Text,
            LegacyValueType::Url,
            LegacyValueType::Select,
            LegacyValueType::Multiselect,
        ] {
            let ft = FieldType::from_legacy(legacy, &LegacyFieldFacets::default());
            assert!(ft.is_text_searchable(), "{legacy:?} must stay searchable");
        }
        for legacy in [
            LegacyValueType::Number,
            LegacyValueType::Boolean,
            LegacyValueType::Date,
        ] {
            let ft = FieldType::from_legacy(legacy, &LegacyFieldFacets::default());
            assert!(
                !ft.is_text_searchable(),
                "{legacy:?} must stay unsearchable"
            );
        }
        // The composite datatypes project structure, not text.
        for dt in [Datatype::Ref, Datatype::Dependent, Datatype::Map] {
            assert!(!FieldType::new(dt).is_text_searchable());
        }
    }

    #[test]
    fn orderable_covers_the_comparable_scalars_only() {
        for dt in [
            Datatype::Number,
            Datatype::Integer,
            Datatype::Date,
            Datatype::DateTime,
        ] {
            assert!(FieldType::new(dt).is_orderable(), "{dt:?}");
        }
        for dt in [
            Datatype::String,
            Datatype::Boolean,
            Datatype::Ref,
            Datatype::Dependent,
            Datatype::Map,
        ] {
            assert!(!FieldType::new(dt).is_orderable(), "{dt:?}");
        }
    }

    #[test]
    fn defaults_apply_when_facets_are_absent() {
        let ft = FieldType::string();
        assert_eq!(ft.effective_cardinality(), Cardinality::Single);
        assert_eq!(ft.effective_value_domain(), ValueDomain::Open);
        assert_eq!(ft.effective_mode(), RefMode::Inline);
        assert!(!ft.is_list());
        assert!(!ft.is_closed());
    }

    // --- Change H (migration) -------------------------------------------

    fn facets_with_values(values: &[&str]) -> LegacyFieldFacets {
        LegacyFieldFacets {
            allowed_values: Some(values.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn migrates_each_legacy_value_type() {
        let none = LegacyFieldFacets::default();
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::String, &none),
            FieldType::string()
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Number, &none),
            FieldType::number()
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Boolean, &none),
            FieldType::boolean()
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Date, &none),
            FieldType::date()
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Url, &none),
            FieldType::uri()
        );
        // `text` always carries an explicit format (plain when not markdown).
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Text, &none),
            FieldType::string().with_format(StringFormat::Plain)
        );
        // `string` + contentFormat markdown gains a format; plain does not.
        let md = LegacyFieldFacets {
            content_format: Some(LegacyContentFormat::Markdown),
            ..Default::default()
        };
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::String, &md),
            FieldType::markdown()
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Text, &md),
            FieldType::markdown()
        );
    }

    #[test]
    fn migrates_select_and_multiselect_with_their_source_set() {
        let f = facets_with_values(&["red", "green"]);
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Select, &f),
            FieldType::select(["red", "green"])
        );
        assert_eq!(
            FieldType::from_legacy(LegacyValueType::Multiselect, &f),
            FieldType::multiselect(["red", "green"])
        );

        let by_ref = LegacyFieldFacets {
            vocabulary_ref: Some("com.test/colours@1".to_string()),
            ..Default::default()
        };
        let migrated = FieldType::from_legacy(LegacyValueType::Select, &by_ref);
        assert_eq!(
            migrated.vocabulary_ref.as_deref(),
            Some("com.test/colours@1")
        );
        assert!(migrated.allowed_values.is_none());
        assert!(migrated.validate().is_empty());
    }

    #[test]
    fn migrates_validation_rules_into_constraints() {
        let facets = LegacyFieldFacets {
            validation_rules: Some(vec![
                LegacyValidationRule {
                    rule_type: "minLength".to_string(),
                    value: Some(json!(3)),
                },
                LegacyValidationRule {
                    rule_type: "pattern".to_string(),
                    value: Some(json!("^[a-z]+$")),
                },
                LegacyValidationRule {
                    rule_type: "required".to_string(),
                    value: None,
                },
            ]),
            ..Default::default()
        };
        let ft = FieldType::from_legacy(LegacyValueType::String, &facets);
        let c = ft.constraints.as_ref().unwrap();
        assert_eq!(c.min_length, Some(3));
        assert_eq!(c.pattern.as_deref(), Some("^[a-z]+$"));
        // `required` is a FieldAssignment concern and must not leak in.
        assert!(c.max_length.is_none());
    }

    #[test]
    fn migration_is_idempotent_in_effect() {
        // Migrating produces a fieldType that itself conforms — the migrated
        // corpus is immediately valid, with no second pass needed.
        for legacy in [
            LegacyValueType::String,
            LegacyValueType::Text,
            LegacyValueType::Number,
            LegacyValueType::Boolean,
            LegacyValueType::Date,
            LegacyValueType::Url,
        ] {
            let ft = FieldType::from_legacy(legacy, &LegacyFieldFacets::default());
            assert!(ft.validate().is_empty(), "{legacy:?} → {ft:?}");
        }
    }

    // --- conformance rules ----------------------------------------------

    #[test]
    fn r2_ref_requires_range_type_and_forbids_it_elsewhere() {
        let mut bare_ref = FieldType::new(Datatype::Ref);
        assert!(bare_ref.validate().iter().any(|v| v.rule == "R2"));

        bare_ref.range_type = Some(ExactTypeRef {
            type_id: "4c000001-0000-4000-a000-000000000001".to_string(),
            type_version: 1,
        });
        assert!(bare_ref.validate().is_empty());

        let mut stray = FieldType::string();
        stray.mode = Some(RefMode::Reference);
        assert!(stray.validate().iter().any(|v| v.rule == "R2"));
    }

    /// srs#534 — the "map-of-`$ref`" capability: `datatype: map` with
    /// `valueRange: ref` shares the `ref` datatype's rangeType/mode contract.
    /// Red-then-green: a bare map+ref is R2-non-conformant (no rangeType);
    /// adding rangeType makes it conformant; the same `rangeType` is still
    /// forbidden for a plain map with a scalar valueRange.
    #[test]
    fn r2_map_of_ref_shares_the_ref_range_contract() {
        let mut bare_map_ref = FieldType::new(Datatype::Map);
        bare_map_ref.value_range = Some(MapValueRange::Ref);
        assert!(
            bare_map_ref.validate().iter().any(|v| v.rule == "R2"),
            "map+ref with no rangeType must be rejected before the fix"
        );

        let range = ExactTypeRef {
            type_id: "4c000001-0000-4000-a000-000000000001".to_string(),
            type_version: 1,
        };
        let with_mode = FieldType::map_of_ref_ids(range.clone());
        assert!(
            with_mode.validate().is_empty(),
            "map+ref+rangeType+mode must validate: {:?}",
            with_mode.validate()
        );
        let inline = FieldType::map_of_ref(range);
        assert!(inline.validate().is_empty(), "{:?}", inline.validate());

        // rangeType stays forbidden for a plain scalar-valued map.
        let mut scalar_map = FieldType::new(Datatype::Map);
        scalar_map.value_range = Some(MapValueRange::String);
        scalar_map.range_type = Some(ExactTypeRef {
            type_id: "4c000001-0000-4000-a000-000000000001".to_string(),
            type_version: 1,
        });
        assert!(scalar_map.validate().iter().any(|v| v.rule == "R2"));
    }

    #[test]
    fn r3_closed_domain_requires_exactly_one_source_set() {
        let mut both = FieldType::select(["a"]);
        both.vocabulary_ref = Some("com.test/v@1".to_string());
        assert!(both.validate().iter().any(|v| v.rule == "R3"));

        let mut neither = FieldType::string();
        neither.value_domain = Some(ValueDomain::Closed);
        assert!(neither.validate().iter().any(|v| v.rule == "R3"));

        let mut orphan_values = FieldType::string();
        orphan_values.allowed_values = Some(vec![AllowedValue::String("a".to_string())]);
        assert!(orphan_values.validate().iter().any(|v| v.rule == "R3"));

        let mut non_string = FieldType::number();
        non_string.value_domain = Some(ValueDomain::Open);
        assert!(non_string.validate().iter().any(|v| v.rule == "R3"));
    }

    /// srs#534 — the "untyped integer enum" capability: `valueDomain: closed`
    /// now also applies to `datatype: integer`. Red-then-green: before the
    /// fix, any non-string closed domain (including integer) was R3-flagged;
    /// after, `FieldType::closed_integer` validates cleanly, while a `number`
    /// (a different scalar datatype, deliberately not extended) still isn't.
    #[test]
    fn r3_integer_closed_domain_is_permitted() {
        let closed_integer = FieldType::closed_integer([0, 2]);
        assert!(
            closed_integer.validate().is_empty(),
            "{:?}",
            closed_integer.validate()
        );
        assert_eq!(
            closed_integer.allowed_values(),
            Some([AllowedValue::Integer(0), AllowedValue::Integer(2)].as_slice())
        );

        let mut non_extended = FieldType::number();
        non_extended.value_domain = Some(ValueDomain::Closed);
        non_extended.allowed_values = Some(vec![AllowedValue::Integer(1)]);
        assert!(non_extended.validate().iter().any(|v| v.rule == "R3"));
    }

    #[test]
    fn r4_item_bounds_require_a_list_and_a_sane_range() {
        let mut single = FieldType::string();
        single.min_items = Some(1);
        assert!(single.validate().iter().any(|v| v.rule == "R4"));

        let mut inverted = FieldType::string().into_list();
        inverted.min_items = Some(3);
        inverted.max_items = Some(1);
        assert!(inverted.validate().iter().any(|v| v.rule == "R4"));

        let mut ok = FieldType::string().into_list();
        ok.min_items = Some(1);
        ok.max_items = Some(3);
        assert!(ok.validate().is_empty());
    }

    #[test]
    fn r6_dependent_requires_depends_on() {
        let bare = FieldType::new(Datatype::Dependent);
        assert!(bare.validate().iter().any(|v| v.rule == "R6"));

        let mut ok = FieldType::new(Datatype::Dependent);
        ok.depends_on = Some("self".to_string());
        assert!(ok.validate().is_empty());

        let mut stray = FieldType::string();
        stray.depends_on = Some("other".to_string());
        assert!(stray.validate().iter().any(|v| v.rule == "R6"));
    }

    #[test]
    fn r9_map_requires_value_range() {
        let bare = FieldType::new(Datatype::Map);
        assert!(bare.validate().iter().any(|v| v.rule == "R9"));

        let mut ok = FieldType::new(Datatype::Map);
        ok.value_range = Some(MapValueRange::Open);
        assert!(ok.validate().is_empty());

        // srs#534 — "ref" is a valid map valueRange too (R9), given rangeType.
        let ok_ref = FieldType::map_of_ref(ExactTypeRef {
            type_id: "4c000001-0000-4000-a000-000000000001".to_string(),
            type_version: 1,
        });
        assert!(ok_ref.validate().is_empty(), "{:?}", ok_ref.validate());

        let mut stray = FieldType::string();
        stray.value_range = Some(MapValueRange::String);
        assert!(stray.validate().iter().any(|v| v.rule == "R9"));
    }

    #[test]
    fn r10_constraints_must_match_the_datatype() {
        let string_bound = FieldType::number().with_constraints(FieldTypeConstraints {
            min_length: Some(1),
            ..Default::default()
        });
        assert!(string_bound.validate().iter().any(|v| v.rule == "R10"));

        let numeric_bound = FieldType::string().with_constraints(FieldTypeConstraints {
            minimum: Some(0.into()),
            ..Default::default()
        });
        assert!(numeric_bound.validate().iter().any(|v| v.rule == "R10"));

        let ok = FieldType::number().with_constraints(FieldTypeConstraints {
            minimum: Some(0.into()),
            maximum: Some(10.into()),
            ..Default::default()
        });
        assert!(ok.validate().is_empty());
    }

    #[test]
    fn r5_format_is_a_string_only_facet() {
        let mut bad = FieldType::number();
        bad.format = Some(StringFormat::Uri);
        assert!(bad.validate().iter().any(|v| v.rule == "R5"));
    }

    // ── RFC-032 Revision 7 conformance predicates ────────────────────────────
    //
    // Every case below is a *constructed* fixture. None of these shapes occurs
    // in the first-party corpus — `cssClassFields` and `predicateFieldId` have
    // zero use sites, all 23 `titleFieldId` sites are eligible under both the
    // old and new readings, and no Tier-2 record uses a `format: uuid` field.
    // A corpus-driven test therefore passes whether or not these predicates
    // exist; only the negative cases below can tell the two apart.

    #[test]
    fn effective_single_is_cardinality_only() {
        // Change-I condition 5 (post-removal): scalar accepted, list rejected.
        // The legacy `FieldAssignment.repeatable` conjunct was removed in the
        // srs#242 Phase-B train — `cardinality` is the sole mechanism, so the
        // flag can no longer defeat eligibility (the trio no longer exists).
        assert!(FieldType::string().is_effective_single());
        assert!(!FieldType::string().into_list().is_effective_single());
    }

    #[test]
    fn i120_r8_text_projection_excludes_uuid_and_email_formats() {
        // The negative case the corpus cannot supply: string-datatyped, and
        // still not searchable, because `format` is load-bearing.
        for excluded in [StringFormat::Uuid, StringFormat::Email] {
            let ft = FieldType::string().with_format(excluded);
            assert_eq!(ft.datatype, Datatype::String, "{excluded:?} is a string");
            assert!(
                !ft.is_text_searchable(),
                "format {excluded:?} must not be text-searchable"
            );
        }
    }

    #[test]
    fn i120_r8_text_projection_admits_the_prose_and_uri_formats() {
        for admitted in [
            None,
            Some(StringFormat::Plain),
            Some(StringFormat::Markdown),
            Some(StringFormat::Uri),
        ] {
            let mut ft = FieldType::string();
            ft.format = admitted;
            assert!(
                ft.is_text_searchable(),
                "format {admitted:?} must be text-searchable"
            );
        }
        // Composites are excluded outright — no recursion into `rangeType`.
        for composite in [Datatype::Ref, Datatype::Dependent, Datatype::Map] {
            assert!(!FieldType::new(composite).is_text_searchable());
        }
    }

    #[test]
    fn i94_r6_conditional_required_rejects_list_cardinality() {
        // Eligible as a single, ineligible the moment either cardinality
        // mechanism makes it repeat.
        assert!(FieldType::string().is_conditional_required_eligible());
        assert!(!FieldType::string()
            .into_list()
            .is_conditional_required_eligible());
    }

    #[test]
    fn i94_r6_conditional_required_admits_only_string_date_and_date_time() {
        for admitted in [Datatype::String, Datatype::Date, Datatype::DateTime] {
            assert!(
                FieldType::new(admitted).is_conditional_required_eligible(),
                "{admitted:?} must be eligible"
            );
        }
        for rejected in [
            Datatype::Number,
            Datatype::Integer,
            Datatype::Boolean,
            Datatype::Ref,
            Datatype::Dependent,
            Datatype::Map,
        ] {
            assert!(
                !FieldType::new(rejected).is_conditional_required_eligible(),
                "{rejected:?} must be ineligible"
            );
        }
    }

    #[test]
    fn t9_theme_css_class_rejects_date_and_non_prose_formats() {
        // `date` and `date-time` serialize as strings on the record, which is
        // why the pre-erratum implementation admitted them. The predicate keys
        // on the declared datatype, not the stored JSON shape.
        for rejected in [Datatype::Date, Datatype::DateTime] {
            assert!(!FieldType::new(rejected).is_theme_css_class_eligible());
        }
        for rejected in [StringFormat::Uri, StringFormat::Uuid, StringFormat::Email] {
            assert!(
                !FieldType::string()
                    .with_format(rejected)
                    .is_theme_css_class_eligible(),
                "format {rejected:?} must not yield a CSS class"
            );
        }
        assert!(!FieldType::string()
            .into_list()
            .is_theme_css_class_eligible());
    }

    #[test]
    fn t9_theme_css_class_admits_both_value_domains() {
        // `[T-9]` does not constrain valueDomain — this is the one place it is
        // laxer than `[N+1]`, and it is deliberate.
        assert!(FieldType::string().is_theme_css_class_eligible());
        assert!(FieldType::closed().is_theme_css_class_eligible());
        assert!(FieldType::markdown().is_theme_css_class_eligible());
    }

    #[test]
    fn n1_title_field_rejects_closed_value_domain() {
        // The negative case that separates `[N+1]` from `[T-9]`.
        assert!(FieldType::string().is_title_field_eligible());
        assert!(
            !FieldType::closed().is_title_field_eligible(),
            "a closed vocabulary is not an eligible heading source"
        );
        assert!(!FieldType::closed_list().is_title_field_eligible());
    }

    #[test]
    fn n1_title_field_rejects_list_and_non_prose_formats() {
        // The shape the `title_field_id_emits_record_heading` fixture used to
        // assert *worked*: a list-cardinality title field.
        assert!(!FieldType::string().into_list().is_title_field_eligible());
        for rejected in [StringFormat::Uri, StringFormat::Uuid, StringFormat::Email] {
            assert!(
                !FieldType::string()
                    .with_format(rejected)
                    .is_title_field_eligible(),
                "format {rejected:?} must not be a heading source"
            );
        }
    }
}
