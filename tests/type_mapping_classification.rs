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
        // A field reference is the field's name, a string at rest, like `callkey`.
        Row { name: "FieldRef", ty: OirType::FieldRef { record: Box::new(OirType::reference("@entity")), value: None }, ts: F, kotlin: F, swift: F },
        // TS Declared `Record<string, FieldValue | undefined>` is the shapeless
        // record — Lossy; Rust's `serde_json::Value` is the one faithful json.
        Row { name: "Json", ty: OirType::Json, ts: L, kotlin: L, swift: L },
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
    assert_eq!(rows.len(), 29, "the table enumerates 29 OirType variants");
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
    assert_eq!(count(|r| r.ts), (16, 12, 1), "TypeScript");
    assert_eq!(count(|r| r.kotlin), (13, 14, 2), "Kotlin");
    assert_eq!(count(|r| r.swift), (13, 14, 2), "Swift");
}

#[test]
fn composition_carries_the_inner_classification() {
    // A faithful container of a lossy element is lossy (a container carries its element's fidelity).
    for (name, cls) in [
        ("ts", orbital_shell_typescript::classify_oir_type(&OirType::array(OirType::Json), None)),
        ("android", orbital_shell_android::classify_oir_type(&OirType::array(OirType::Json))),
        ("ios", orbital_shell_ios::classify_oir_type(&OirType::array(OirType::Json))),
    ] {
        assert_eq!(cls, L, "{name}: Array<Json>");
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
