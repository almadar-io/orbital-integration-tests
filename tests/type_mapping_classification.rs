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
//! Post-Phase-3D summary: TS 15/10/1, Kotlin 12/12/2, Swift 13/11/2,
//! Python 15/9/2, Rust 13/11/2 (faithful/widening/lossy). Classification is
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
    python: TypeMappingClass,
    rust: TypeMappingClass,
}

fn struct_variant(tag: &str) -> OirField {
    OirField::new(tag, OirType::Struct {
        fields: vec![OirField::new("value", OirType::String)],
    })
}

fn table() -> Vec<Row> {
    vec![
        Row { name: "String", ty: OirType::String, ts: F, kotlin: F, swift: F, python: F, rust: F },
        Row { name: "Number", ty: OirType::Number, ts: F, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Integer", ty: OirType::Integer, ts: W, kotlin: W, swift: F, python: F, rust: F },
        Row { name: "Boolean", ty: OirType::Boolean, ts: F, kotlin: F, swift: F, python: F, rust: F },
        // Python is the one faithful `datetime` arm; every other target
        // carries dates as strings (TS `string | number` — honest about the
        // at-rest domain, still a widening per the table's judgement call).
        Row { name: "Date", ty: OirType::Date, ts: W, kotlin: W, swift: W, python: F, rust: W },
        // DateTime renders via the `date` arm in every backend, so it
        // inherits the `date` classification everywhere.
        Row { name: "DateTime", ty: OirType::DateTime, ts: W, kotlin: W, swift: W, python: F, rust: W },
        // The semantic string domains render via the faithful string arm but
        // erase the domain — Widening in every shell.
        Row { name: "Email", ty: OirType::Email, ts: W, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Url", ty: OirType::Url, ts: W, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Phone", ty: OirType::Phone, ts: W, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Uuid", ty: OirType::Uuid, ts: W, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Image", ty: OirType::Image, ts: W, kotlin: W, swift: W, python: W, rust: W },
        // Money renders via `number` but erases currency semantics — W
        // even where `number` itself is Faithful (TS).
        Row { name: "Money", ty: OirType::Money, ts: W, kotlin: W, swift: W, python: W, rust: W },
        // `file` renders as the target's struct arm of `file_value_fields()` —
        // Faithful wherever that arm is (TS/Kotlin/Swift already; Python/Rust
        // since the Phase-2 named-struct fix), Lossy only where the struct arm
        // still erases (none today).
        Row { name: "File", ty: OirType::File, ts: F, kotlin: F, swift: F, python: F, rust: F },
        // TS Declared `Record<string, FieldValue | undefined>` is the shapeless
        // record — Lossy; Rust's `serde_json::Value` is the one faithful json.
        Row { name: "Json", ty: OirType::Json, ts: L, kotlin: L, swift: L, python: L, rust: F },
        // Phase 3: TS has always spelled the closed union; Kotlin
        // (JsonPrimitive), Swift (4-case Codable enum), Python
        // (`Union[str, float, bool, None]`), and Rust (`#[serde(untagged)]`
        // enum) spell it now too — all four flipped Lossy→Faithful.
        // TS has the canonical `PatternValue` (Faithful). Phase 3: the json
        // carrier preserves the tree STRUCTURE and erases only the pattern
        // TYPE — Widening in Kotlin/Swift/Python; Rust keeps Lossy per the
        // Phase-3 disposition (typed PatternNode deferred, rides on
        // named_types).
        Row { name: "Node", ty: OirType::Node, ts: F, kotlin: W, swift: W, python: W, rust: L },
        // Containers carry their element's fidelity (all-faithful elements
        // here; composed cases are covered in `composition_*` below).
        Row { name: "Array", ty: OirType::array(OirType::String), ts: F, kotlin: F, swift: F, python: F, rust: F },
        Row { name: "Map", ty: OirType::map(OirType::String, OirType::String), ts: F, kotlin: F, swift: F, python: F, rust: F },
        Row { name: "Optional", ty: OirType::optional(OirType::String), ts: F, kotlin: F, swift: F, python: F, rust: F },
        // Declared relation fields store the id in every shell (core default `string`).
        Row { name: "Reference", ty: OirType::reference("User"), ts: W, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "Enum", ty: OirType::enum_type("Status"), ts: F, kotlin: F, swift: F, python: F, rust: F },
        Row { name: "Trait", ty: OirType::Trait, ts: F, kotlin: W, swift: W, python: W, rust: W },
        Row { name: "SExpr", ty: OirType::SExpr, ts: F, kotlin: L, swift: L, python: L, rust: L },
        // Phase 3D: every shell with a named-type facility (all five) hoists
        // the entry's declaration from `named_types` and references it by
        // bare name — Faithful everywhere, TS from the start, the other four
        // from the Phase-3D shell-readiness flip.
        Row { name: "TypeRef", ty: OirType::TypeRef { name: "TreeNode".to_string() }, ts: F, kotlin: F, swift: F, python: F, rust: F },
        // Phase 2: Struct is a named hoisted declaration in every shell now —
        // Python/Rust flipped Lossy→Faithful with the named-struct fix.
        Row {
            name: "Struct",
            ty: OirType::Struct { fields: vec![OirField::new("name", OirType::String)] },
            ts: F,
            kotlin: F,
            swift: F,
            python: F,
            rust: F,
        },
        // Phase 2: TS spells a real discriminated union; Kotlin (sealed
        // class), Swift (associated-value enum), Python (typing.Union of named
        // models), and Rust (untagged enum over named structs) do too — all
        // four flipped Lossy→Faithful.
        Row {
            name: "Union",
            ty: OirType::Union { variants: vec![struct_variant("a"), struct_variant("b")] },
            ts: F,
            kotlin: F,
            swift: F,
            python: F,
            rust: F,
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
        assert_eq!(
            orbital_shell_python::classify_oir_type(&row.ty),
            row.python,
            "Python: {}",
            row.name
        );
        assert_eq!(
            orbital_shell_rust::codegen::entities::classify_oir_type(&row.ty),
            row.rust,
            "Rust: {}",
            row.name
        );
    }
}

#[test]
fn per_shell_counts_match_the_table_summary() {
    let rows = table();
    assert_eq!(rows.len(), 25, "the table enumerates 25 OirType variants");
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
    // flipped Lossy→Faithful in Kotlin/Swift/Python/Rust).
    assert_eq!(count(|r| r.ts), (14, 10, 1), "TypeScript");
    assert_eq!(count(|r| r.kotlin), (10, 13, 2), "Kotlin");
    assert_eq!(count(|r| r.swift), (11, 12, 2), "Swift");
    assert_eq!(count(|r| r.python), (13, 10, 2), "Python");
    assert_eq!(count(|r| r.rust), (12, 11, 2), "Rust");
}

#[test]
fn composition_carries_the_inner_classification() {
    // A faithful container of a lossy element is lossy — except in Rust,
    // whose `Json` carrier is the one faithful json arm, so `Vec<serde_json::Value>`
    // stays Faithful there (a container carries its element's fidelity).
    for (name, cls) in [
        ("ts", orbital_shell_typescript::classify_oir_type(&OirType::array(OirType::Json), None)),
        ("android", orbital_shell_android::classify_oir_type(&OirType::array(OirType::Json))),
        ("ios", orbital_shell_ios::classify_oir_type(&OirType::array(OirType::Json))),
        ("python", orbital_shell_python::classify_oir_type(&OirType::array(OirType::Json))),
    ] {
        assert_eq!(cls, L, "{name}: Array<Json>");
    }
    assert_eq!(
        orbital_shell_rust::codegen::entities::classify_oir_type(&OirType::array(OirType::Json)),
        F,
        "rust: Array<Json> (faithful json carrier)"
    );
    // Optional<Money> widens (currency erased) without becoming lossy.
    assert_eq!(
        orbital_shell_typescript::classify_oir_type(&OirType::optional(OirType::Money), None),
        W
    );
}

#[test]
fn rust_map_key_coercion_matches_the_rendering() {
    use orbital_shell_rust::codegen::entities::classify_oir_type as rust_cls;
    // String keys keep the value's class (HashMap<String, V>).
    assert_eq!(
        rust_cls(&OirType::map(OirType::String, OirType::String)),
        F
    );
    // Integer/Number keys are coerced to `String` — Widening at best.
    assert_eq!(
        rust_cls(&OirType::map(OirType::Integer, OirType::String)),
        W
    );
    // Any other key collapses the whole map to `serde_json::Value` — Lossy.
    assert_eq!(
        rust_cls(&OirType::map(OirType::Boolean, OirType::String)),
        L
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
    assert_eq!(
        orbital_shell_rust::codegen::entities::classify_oir_type(&OirType::String),
        F
    );
}
