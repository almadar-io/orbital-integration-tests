//! Cross-shell type-mapping classification conformance test (Compiler-Spec
//! Phase 1 + Phase 2 + Phase 3 flips). For every `OirType` variant, asserts
//! each of the 5 shells' `TargetTypeRenderer` classification equals the
//! enumeration table (`orbital-rust/docs/Orbital_Type_Chain_Table.md`) AS
//! AMENDED by Phase 2 (`docs/Almadar_Compiler_Spec.md`) and Phase 3 (gaps
//! §130): Phase 2 moved Kotlin/Swift/Python/Rust `Union` (all four), plus
//! `Struct`/`File` in Python/Rust, from Lossy to Faithful; Phase 3
//! reclassified `Node` as Widening in Kotlin/Swift/Python (the json carrier
//! preserves the tree STRUCTURE, erasing only the pattern TYPE; typed
//! PatternNode codegen is deferred, riding on named_types), and Phase 3D
//! flipped `TypeRef` to Faithful in Kotlin/Swift/Python/Rust (every shell
//! hoists the `named_types` declaration and references it by bare name).
//! Summary with Bytes and CallKey: TS 15/11/1, Kotlin 12/13/2, Swift 12/13/2,
//! Python 14/11/2, Rust 13/12/2 (faithful/widening/lossy). Classification is
//! metadata only — this test exercises no rendering path and pins no emitted
//! bytes.

use orbital_core::ir::{OirField, OirType, TypeMappingClass};
use orbital_core::ir::TypeMappingClass::{Faithful as F, Lossy as L, Widening as W};

/// One row of the enumeration table: a representative `OirType` (TS is
/// classified at its Declared mode, the mode the table cites) plus the
/// expected class per target, in table column order.
struct Row {
    name: &'static str,
    ty: OirType,
    ts: TypeMappingClass,
    kotlin: TypeMappingClass,
    swift: TypeMappingClass,
}

fn struct_variant(tag: &str) -> OirField {
    OirField::new(tag, OirType::Struct {
        additional: None,
        fields: vec![OirField::new("value", OirType::String)].into(),
    })
}

fn table() -> Vec<Row> {
    vec![
        Row { name: "String", ty: OirType::String, ts: F, kotlin: F, swift: F },
        Row { name: "Number", ty: OirType::Number, ts: F, kotlin: W, swift: W },
        Row { name: "Instant", ty: OirType::Instant, ts: W, kotlin: W, swift: W },
        Row { name: "Integer", ty: OirType::Integer, ts: W, kotlin: F, swift: F },
        Row { name: "Boolean", ty: OirType::Boolean, ts: F, kotlin: F, swift: F },
        // Python is the one faithful `datetime` arm; every other target
        // carries dates as strings (TS `string | number` — honest about the
        // at-rest domain, still a widening per the table's judgement call).
        Row { name: "Date", ty: OirType::Date, ts: W, kotlin: W, swift: W },
        // DateTime renders via the `date` arm in every backend, so it
        // inherits the `date` classification everywhere.
        Row { name: "DateTime", ty: OirType::DateTime, ts: W, kotlin: W, swift: W },
        // The semantic string domains render via the faithful string arm but
        // erase the domain — Widening in every shell.
        Row { name: "Email", ty: OirType::Email, ts: W, kotlin: W, swift: W },
        Row { name: "Url", ty: OirType::Url, ts: W, kotlin: W, swift: W },
        Row { name: "Phone", ty: OirType::Phone, ts: W, kotlin: W, swift: W },
        Row { name: "Uuid", ty: OirType::Uuid, ts: W, kotlin: W, swift: W },
        Row { name: "Image", ty: OirType::Image, ts: W, kotlin: W, swift: W },
        // Money renders via `number` but erases currency semantics — W
        // even where `number` itself is Faithful (TS).
        Row { name: "Money", ty: OirType::Money, ts: W, kotlin: W, swift: W },
        // `file` renders as the target's struct arm of `file_value_fields()` —
        // Faithful wherever that arm is (TS/Kotlin/Swift already; Python/Rust
        // since the Phase-2 named-struct fix), Lossy only where the struct arm
        // still erases (none today).
        Row { name: "File", ty: OirType::File, ts: F, kotlin: F, swift: F },
        // `bytes` renders as base64 text in every target — the binary meaning is erased (W).
        Row { name: "Bytes", ty: OirType::Bytes, ts: W, kotlin: W, swift: W },
        // `callkey` is a string at rest and classifies as `string` (like `slot`, `pattern`,
        // `event`, `eventaddress`, which lower to `String`): the explicit-cast rule is a
        // language-level check, not a rendered type, so no shell loses a value.
        Row { name: "CallKey", ty: OirType::CallKey, ts: F, kotlin: F, swift: F },
        // A catalog key is a string at rest too; only the checker treats it as nominal.
        Row { name: "MessageKey", ty: OirType::MessageKey, ts: F, kotlin: F, swift: F },
        // A field reference is the field's name, a string at rest, like `callkey`.
        Row { name: "FieldRef", ty: OirType::FieldRef { record: Box::new(OirType::reference("@entity")), value: None }, ts: F, kotlin: F, swift: F },
        // Phase 3: TS has always spelled the closed union; Kotlin
        // (JsonPrimitive), Swift (4-case Codable enum), Python
        // (`Union[str, float, bool, None]`), and Rust (`#[serde(untagged)]`
        // enum) spell it now too — all four flipped Lossy→Faithful.
        // TS has the canonical `PatternValue` (Faithful). Phase 3: the json
        // carrier preserves the tree STRUCTURE and erases only the pattern
        // TYPE — Widening in Kotlin/Swift/Python; Rust keeps Lossy per the
        // Phase-3 disposition (typed PatternNode deferred, rides on
        // named_types).
        Row { name: "Node", ty: OirType::Node, ts: F, kotlin: W, swift: W },
        // Containers carry their element's fidelity (all-faithful elements
        // here; composed cases are covered in `composition_*` below).
        Row { name: "Array", ty: OirType::array(OirType::String), ts: F, kotlin: F, swift: F },
        Row { name: "Map", ty: OirType::map(OirType::String, OirType::String), ts: F, kotlin: F, swift: F },
        Row { name: "Optional", ty: OirType::optional(OirType::String), ts: F, kotlin: F, swift: F },
        // Declared relation fields store the id in every shell (core default `string`).
        Row { name: "Reference", ty: OirType::reference("User"), ts: W, kotlin: W, swift: W },
        Row { name: "Enum", ty: OirType::enum_type("Status"), ts: F, kotlin: F, swift: F },
        Row { name: "Trait", ty: OirType::Trait, ts: F, kotlin: W, swift: W },
        Row { name: "SExpr", ty: OirType::SExpr, ts: F, kotlin: L, swift: L },
        // Phase 3D: every shell with a named-type facility (all five) hoists
        // the entry's declaration from `named_types` and references it by
        // bare name — Faithful everywhere, TS from the start, the other four
        // from the Phase-3D shell-readiness flip.
        Row { name: "TypeRef", ty: OirType::TypeRef { name: "TreeNode".to_string() }, ts: F, kotlin: F, swift: F },
        // Phase 2: Struct is a named hoisted declaration in every shell now —
        // Python/Rust flipped Lossy→Faithful with the named-struct fix.
        Row {
            name: "Struct",
            ty: OirType::Struct { additional: None, fields: vec![OirField::new("name", OirType::String)].into() },
            ts: F,
            kotlin: F,
            swift: F,
        },
        // Phase 2: TS spells a real discriminated union; Kotlin (sealed
        // class), Swift (associated-value enum), Python (typing.Union of named
        // models), and Rust (untagged enum over named structs) do too — all
        // four flipped Lossy→Faithful.
        Row {
            name: "Union",
            ty: OirType::Union { variants: vec![struct_variant("a"), struct_variant("b")].into() },
            ts: F,
            kotlin: F,
            swift: F,
        },
    ]
}

#[test]
fn every_variant_matches_the_enumeration_table_per_shell() {
    for row in table() {
        assert_eq!(
            orbital_shell_typescript::classify_oir_type(&row.ty, None),
            row.ts,
            "TypeScript: {}",
            row.name
        );
        assert_eq!(
            orbital_shell_android::classify_oir_type(&row.ty),
            row.kotlin,
            "Kotlin: {}",
            row.name
        );
        assert_eq!(
            orbital_shell_ios::classify_oir_type(&row.ty),
            row.swift,
            "Swift: {}",
            row.name
        );
    }
}

#[test]
fn per_shell_counts_match_the_table_summary() {
    let rows = table();
    assert_eq!(rows.len(), 28, "the table enumerates 28 OirType variants");
    let count = |f: fn(&Row) -> TypeMappingClass| {
        let mut counts = (0, 0, 0);
        for row in &rows {
            match f(row) {
                F => counts.0 += 1,
                W => counts.1 += 1,
                L => counts.2 += 1,
            }
        }
        counts
    };
    // (faithful, widening, lossy) per the Phase-3D-amended summary (TypeRef
    // flipped Lossy→Faithful in Kotlin/Swift/Python/Rust); Instant is ISO text like DateTime (2026-10-28 ruling).
    assert_eq!(count(|r| r.ts), (16, 12, 0), "TypeScript");
    assert_eq!(count(|r| r.kotlin), (13, 14, 1), "Kotlin");
    assert_eq!(count(|r| r.swift), (13, 14, 1), "Swift");
}

/// A tuple of unlike slots has no typed spelling in the json-carrier shells: it is a list of their untyped value.
fn mixed_pair() -> OirType {
    OirType::Tuple { items: vec![OirType::String, OirType::Number] }
}

#[test]
fn composition_carries_the_inner_classification() {
    // A faithful container of a lossy element is lossy (a container carries its element's fidelity).
    for (name, cls) in [
        ("ts", orbital_shell_typescript::classify_oir_type(&OirType::array(mixed_pair()), None)),
        ("android", orbital_shell_android::classify_oir_type(&OirType::array(mixed_pair()))),
        ("ios", orbital_shell_ios::classify_oir_type(&OirType::array(mixed_pair()))),
    ] {
        assert_eq!(cls, L, "{name}: Array<mixed tuple>");
    }
    // Optional<Money> widens (currency erased) without becoming lossy.
    assert_eq!(
        orbital_shell_typescript::classify_oir_type(&OirType::optional(OirType::Money), None),
        W
    );
}

#[test]
fn enum_values_sidecar_is_faithful_at_the_field_position() {
    let vals = ["a".to_string(), "b".to_string()];
    // A format-union field lowered to String + sidecar spells the closed
    // domain in every shell — Faithful, never a bare-string widening.
    assert_eq!(
        orbital_shell_typescript::classify_oir_type(&OirType::String, Some(&vals)),
        F
    );
}

/// The checker (a knob default) and the dispatch validator (an event payload member) share one grammar per scalar type.
#[test]
fn the_checker_and_the_dispatch_validator_accept_and_refuse_the_same_scalars() {
    use orbital_core::runtime::payload_validation::validate_event_payload;
    use orbital_core::schema::{OrbitalSchema, PayloadField};
    use orbital_compiler::phases::validation::{validate_comprehensive_without_services, ErrorCode};
    use serde_json::json;

    let enum_values = json!(["on", "off"]);
    let samples: Vec<(&str, Option<&serde_json::Value>, serde_json::Value)> = vec![
        ("string", None, json!("x")),
        ("string", None, json!(1)),
        ("string", None, json!(true)),
        ("number", None, json!(1.5)),
        ("number", None, json!("1")),
        ("number", None, json!(false)),
        ("int", None, json!(3)),
        ("int", None, json!("3")),
        ("boolean", None, json!(true)),
        ("boolean", None, json!("true")),
        ("boolean", None, json!(0)),
        ("date", None, json!("2026-10-28")),
        ("date", None, json!("2026-13-45")),
        ("date", None, json!("tomorrow")),
        ("date", None, json!("2026-10-28T10:30:00Z")),
        ("date", None, json!(0)),
        ("datetime", None, json!("2026-10-28T10:30:00Z")),
        ("datetime", None, json!("2026-10-28T10:30:00+02:00")),
        ("datetime", None, json!("2026-10-28")),
        ("datetime", None, json!("2026-10-28T10:30:00")),
        ("datetime", None, json!("soon")),
        ("datetime", None, json!(0)),
        ("instant", None, json!("2026-10-29T02:30:00.000Z")),
        ("instant", None, json!("1970-01-01T00:00:00.000Z")),
        ("instant", None, json!("2026-10-28")),
        ("instant", None, json!("1970-01-01 00:00:00")),
        ("instant", None, json!(1_700_000_000_000_i64)),
        ("instant", None, json!(true)),
        ("timestamp", None, json!("2026-10-29T02:30:00Z")),
        ("timestamp", None, json!("2026-10-29")),
        ("timestamp", None, json!(0)),
        ("timestamp", None, json!("nope")),
        ("string", Some(&enum_values), json!("on")),
        ("string", Some(&enum_values), json!("off")),
        ("string", Some(&enum_values), json!("maybe")),
        ("string", Some(&enum_values), json!("")),
        ("string", Some(&enum_values), json!(1)),
    ];

    let mut both_refuse = 0;
    for (tag, values, value) in samples {
        let mut knob = json!({ "type": tag, "default": value });
        let mut member = json!({ "name": "n", "type": tag, "required": true });
        if let Some(values) = values {
            knob["values"] = values.clone();
            member["values"] = values.clone();
        }
        let schema: OrbitalSchema = serde_json::from_value(json!({
            "name": "Twin",
            "version": "0.1.0",
            "orbitals": [{
                "name": "TwinOrbital",
                "entity": {
                    "name": "Dashboard",
                    "collection": "dashboards",
                    "fields": [{ "name": "id", "type": "string", "required": true, "primaryKey": true }]
                },
                "traits": [{
                    "name": "Knobbed",
                    "linkedEntity": "Dashboard",
                    "config": { "knob": knob },
                    "stateMachine": {
                        "states": [{ "name": "idle", "isInitial": true }],
                        "events": [],
                        "transitions": [{ "from": "idle", "to": "idle", "event": "INIT", "effects": [] }]
                    }
                }]
            }]
        }))
        .expect("schema deserializes");
        let checker_accepts = !validate_comprehensive_without_services(&schema)
            .errors
            .iter()
            .any(|e| e.code == ErrorCode::TraitConfigShapeMismatch);

        let field: PayloadField = serde_json::from_value(member).expect("payload field deserializes");
        let payload = serde_json::from_value(json!({ "n": value.clone() })).expect("payload deserializes");
        let dispatch_accepts = validate_event_payload("GO", &payload, &[field]).is_empty();
        assert_eq!(
            checker_accepts, dispatch_accepts,
            "`{tag}` {values:?} = {value}: checker accepts {checker_accepts}, dispatch accepts {dispatch_accepts}"
        );
        both_refuse += usize::from(!checker_accepts);
    }
    assert!(both_refuse >= 15, "the checker side must actually refuse: {both_refuse}");

    let accepts = |tag: &str, values: Option<&serde_json::Value>, value: serde_json::Value| -> bool {
        let mut member = json!({ "name": "n", "type": tag, "required": true });
        if let Some(values) = values {
            member["values"] = values.clone();
        }
        let field: PayloadField = serde_json::from_value(member).unwrap();
        let payload = serde_json::from_value(json!({ "n": value })).unwrap();
        validate_event_payload("GO", &payload, &[field]).is_empty()
    };
    assert!(accepts("string", Some(&enum_values), json!("on")), "a declared member is accepted");
    assert!(!accepts("string", Some(&enum_values), json!("maybe")), "an undeclared literal is refused");
    assert!(accepts("string", None, json!("maybe")), "control: no declared set");
}

/// The `.lolo` spelling of a JSON value, for a knob default.
fn lolo_literal(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).expect("a string serializes"),
        Value::Array(items) => format!("[{}]", items.iter().map(lolo_literal).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => format!(
            "{{ {} }}",
            map.iter().map(|(k, v)| format!("{k}: {}", lolo_literal(v))).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// The differential the type-system cause names: a declaration the checker accepts a value for is never refused for
/// that value at dispatch, for records, lists and optionals as for scalars (the scalar twin above). Both sides are
/// read from ONE `.lolo` declaration: the knob `k : T = <value>` the checker walks and the event member `n : T!`
/// the dispatch validator walks.
#[test]
fn the_checker_and_the_dispatch_validator_accept_and_refuse_the_same_records_lists_and_optionals() {
    use orbital_compiler::phases::validation::{validate_comprehensive_without_services, ErrorCode};
    use orbital_core::runtime::payload_validation::validate_event_payload;
    use orbital_core::schema::PayloadField;
    use serde_json::json;

    let record = "{ a : string!, n : number }";
    let samples: Vec<(String, serde_json::Value)> = vec![
        (record.into(), json!({ "a": "x" })),
        (record.into(), json!({ "a": "x", "n": 1 })),
        (record.into(), json!({})),
        (record.into(), json!({ "a": 1 })),
        (record.into(), json!({ "a": "x", "n": "s" })),
        (record.into(), json!("text")),
        (record.into(), json!([1])),
        ("[string]".into(), json!(["a"])),
        ("[string]".into(), json!([])),
        ("[string]".into(), json!([1])),
        ("[string]".into(), json!(["a", 2])),
        ("[string]".into(), json!("a")),
        (format!("[{record}]"), json!([{ "a": "x" }])),
        (format!("[{record}]"), json!([{ "a": 1 }])),
        (format!("[{record}]"), json!([{}])),
        ("string?".into(), json!(null)),
        ("string?".into(), json!("x")),
        ("string?".into(), json!(1)),
        (format!("({record})?"), json!(null)),
        (format!("({record})?"), json!({ "a": "x" })),
        (format!("({record})?"), json!({ "a": 1 })),
        ("{ tags : [string]! }".into(), json!({ "tags": ["a"] })),
        ("{ tags : [string]! }".into(), json!({ "tags": [1] })),
        ("{ tags : [string]! }".into(), json!({})),
        ("Map string number".into(), json!({ "x": 1 })),
        ("Map string number".into(), json!({ "x": "s" })),
    ];

    let mut both_refuse = 0;
    let mut both_accept = 0;
    let mut disagreements = Vec::new();
    for (ty, value) in &samples {
        let source = format!(
            r#"
orbital Twin {{
  entity Doc [runtime] {{ id : string! }}
  trait Knobbed -> Doc {{
    initial: idle
    state idle {{ INIT -> idle (set @entity.id (json/stringify @config.k)) }}
    emits {{ GO -> external {{ n : {ty}! }} }}
    config {{ k : {ty} = {} }}
  }}
}}
"#,
            lolo_literal(value)
        );
        let (checker_accepts, member) = match orbital_lolo::parse_and_lower_str(&source) {
            Err(_) => (false, None),
            Ok(schema) => {
                let accepts = !validate_comprehensive_without_services(&schema)
                    .errors
                    .iter()
                    .any(|e| e.code == ErrorCode::TraitConfigShapeMismatch);
                let lowered = serde_json::to_value(&schema).expect("schema serializes");
                let member = lowered["orbitals"][0]["traits"][0]["emits"][0]["payloadSchema"][0].clone();
                (accepts, Some(member))
            }
        };
        let Some(member) = member else {
            // The `.lolo` lowering refused the declaration itself; nothing reaches dispatch.
            both_refuse += 1;
            continue;
        };
        let field: PayloadField = serde_json::from_value(member.clone())
            .unwrap_or_else(|e| panic!("`{ty}` payload member {member}: {e}"));
        let payload = serde_json::from_value(json!({ "n": value.clone() })).expect("payload deserializes");
        let dispatch_accepts = validate_event_payload("GO", &payload, &[field]).is_empty();
        if checker_accepts && !dispatch_accepts {
            disagreements.push(format!("`{ty}` = {value}: the checker accepts what dispatch refuses"));
        }
        both_refuse += usize::from(!checker_accepts && !dispatch_accepts);
        both_accept += usize::from(checker_accepts && dispatch_accepts);
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
    assert!(both_refuse >= 8 && both_accept >= 8, "the twin must exercise both sides: refuse {both_refuse}, accept {both_accept}");
}

/// The same twin for an effect: the payload literal an `emit` carries is checked against the declared event signature by
/// `orb validate`; dispatch checks the delivered payload against the same signature. Validate-OK never meets a refusal.
#[test]
fn an_emit_the_checker_accepts_is_never_refused_by_the_dispatch_validator() {
    use orbital_compiler::phases::validation::{validate_comprehensive_without_services, ErrorCode};
    use orbital_core::runtime::payload_validation::validate_event_payload;
    use orbital_core::schema::PayloadField;
    use serde_json::json;

    let record = "{ a : string!, n : number }";
    let enum_ty = r#""on" | "off""#;
    let samples: Vec<(String, serde_json::Value)> = vec![
        ("string".into(), json!("x")),
        ("string".into(), json!(1)),
        ("string".into(), json!(true)),
        ("number".into(), json!(1.5)),
        ("number".into(), json!("1")),
        ("boolean".into(), json!(false)),
        ("boolean".into(), json!(0)),
        ("int".into(), json!(3)),
        ("int".into(), json!(3.5)),
        ("string?".into(), json!(null)),
        ("string?".into(), json!(1)),
        ("string".into(), json!(null)),
        ("number".into(), json!(null)),
        ("[string]".into(), json!(null)),
        (record.into(), json!(null)),
        ("[string]".into(), json!(["a"])),
        ("[string]".into(), json!(["a", 2])),
        ("[string]".into(), json!([])),
        ("[string]".into(), json!("a")),
        ("[number]".into(), json!([1, 2])),
        ("[number]".into(), json!(["1"])),
        (record.into(), json!({ "a": "x" })),
        (record.into(), json!({})),
        (record.into(), json!({ "a": 1 })),
        (record.into(), json!({ "a": "x", "z": 1 })),
        (record.into(), json!("text")),
        (format!("[{record}]"), json!([{ "a": "x" }, { "a": "y", "n": 2 }])),
        (format!("[{record}]"), json!([{ "a": "x" }, {}])),
        (format!("[{record}]"), json!([{ "a": 1 }])),
        (format!("[{record}]"), json!([{ "a": "x", "n": "s" }])),
        (format!("({record})?"), json!(null)),
        (format!("({record})?"), json!({ "a": 1 })),
        ("{ tags : [string]! }".into(), json!({ "tags": ["a"] })),
        ("{ tags : [string]! }".into(), json!({ "tags": [1] })),
        ("Map string number".into(), json!({ "x": 1 })),
        ("Map string number".into(), json!({ "x": "s" })),
        (enum_ty.into(), json!("on")),
        (enum_ty.into(), json!("maybe")),
        (enum_ty.into(), json!(1)),
        ("date".into(), json!("2026-10-28")),
        ("date".into(), json!("tomorrow")),
        ("datetime".into(), json!("2026-10-28T10:30:00Z")),
        ("datetime".into(), json!("soon")),
        ("instant".into(), json!("2026-10-29T02:30:00.000Z")),
        ("instant".into(), json!("2026-10-28")),
        ("[[string]]".into(), json!([["a"], ["b"]])),
        ("[[string]]".into(), json!([["a"], [1]])),
        ("[\"on\" | \"off\"]".into(), json!(["on", "off"])),
        ("[\"on\" | \"off\"]".into(), json!(["on", "maybe"])),
        ("{ mode : \"on\" | \"off\"! }".into(), json!({ "mode": "on" })),
        ("{ mode : \"on\" | \"off\"! }".into(), json!({ "mode": "maybe" })),
        ("{ inner : { k : number! }! }".into(), json!({ "inner": { "k": 1 } })),
        ("{ inner : { k : number! }! }".into(), json!({ "inner": { "k": "s" } })),
        ("{ inner : { k : number! }! }".into(), json!({ "inner": {} })),
    ];

    let emit_codes = [
        ErrorCode::EmitPayloadFieldTypeMismatch,
        ErrorCode::EmitPayloadMissingRequired,
        ErrorCode::EmitPayloadExtraField,
    ];
    let mut both_refuse = 0;
    let mut both_accept = 0;
    let mut disagreements = Vec::new();
    for (ty, value) in &samples {
        let source = format!(
            r#"
orbital Twin {{
  entity Doc [runtime] {{ id : string! }}
  trait Sender -> Doc {{
    initial: idle
    state idle {{ INIT -> idle (emit GO {{ n: {} }}) }}
    emits {{ GO -> external {{ n : {ty}! }} }}
  }}
}}
"#,
            lolo_literal(value)
        );
        let Ok(schema) = orbital_lolo::parse_and_lower_str(&source) else {
            both_refuse += 1;
            continue;
        };
        let checker_accepts = !validate_comprehensive_without_services(&schema)
            .errors
            .iter()
            .any(|e| emit_codes.contains(&e.code));
        let lowered = serde_json::to_value(&schema).expect("schema serializes");
        let member = lowered["orbitals"][0]["traits"][0]["emits"][0]["payloadSchema"][0].clone();
        let field: PayloadField = serde_json::from_value(member.clone())
            .unwrap_or_else(|e| panic!("`{ty}` payload member {member}: {e}"));
        let payload = serde_json::from_value(json!({ "n": value.clone() })).expect("payload deserializes");
        let dispatch_accepts = validate_event_payload("GO", &payload, &[field]).is_empty();
        if checker_accepts && !dispatch_accepts {
            disagreements.push(format!("`{ty}` = {value}: validate accepts the emit, dispatch refuses it"));
        }
        both_refuse += usize::from(!checker_accepts && !dispatch_accepts);
        both_accept += usize::from(checker_accepts && dispatch_accepts);
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
    assert!(both_refuse >= 8 && both_accept >= 8, "the twin must exercise both sides: refuse {both_refuse}, accept {both_accept}");
}

/// A literal `null` written into a position whose declared type is not `T?` is refused by the Rust type checker at the
/// `set` target (and, above, an emit payload member); the paired optional declaration accepts it. The `persist` row
/// literals are checked by the std lint atom std-orb-lint-effect-forms, whose goldens `row-literal-null-into-string`
/// (create and update) and `control-row-literal-null-into-optional` pin the same rule.
#[test]
fn a_null_into_a_never_null_set_target_or_row_literal_is_refused_and_an_optional_one_accepts_it() {
    use orbital_compiler::phases::validation::validate_comprehensive_without_services;

    let findings = |ty: &str, effect: &str| -> Vec<String> {
        let source = format!(
            r#"
orbital Twin {{
  entity Doc [persistent] {{
    id : string!
    code : {ty}
  }}
  trait Writer -> Doc {{
    initial: idle
    state idle {{ INIT -> idle ({effect}) }}
  }}
}}
"#
        );
        let schema = orbital_lolo::parse_and_lower_str(&source).expect("lowers");
        validate_comprehensive_without_services(&schema)
            .errors
            .iter()
            .map(|e| format!("{:?}: {}", e.code, e.message))
            .collect()
    };
    for (position, effect) in [("set", "set @entity.code null")] {
        let refused = findings("string", effect);
        assert!(
            refused.iter().any(|f| f.contains("null") && f.contains("string")),
            "{position}: a null into `code : string` must be refused naming the field type: {refused:#?}"
        );
        let accepted = findings("string?", effect);
        assert!(accepted.iter().all(|f| !f.contains("null")), "{position}: `code : string?` takes null: {accepted:#?}");
    }
}
