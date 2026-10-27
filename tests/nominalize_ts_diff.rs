//! Phase-3D TS acceptance harness: emit a representative module through the
//! TypeScript backend with nominalization OFF (today's default path) vs ON
//! (`nominalize_module`), capture both `entities.ts` outputs under /tmp, and
//! assert the EXPECTED Phase-3D shape: every anonymous struct renders as a
//! hoisted named interface with identical field content — structurally
//! identical types, byte-different text. The owner reviews the /tmp diff.

use orbital_compiler::phases::nominalize::nominalize_module;
use orbital_core::generate::Backend;
use orbital_core::ir::{OirEntity, OirField, OirModule, OirType};
use orbital_shell_typescript::TypeScriptBackend;

fn fixture_module() -> OirModule {
    let mut entity = OirEntity::new("Order");
    entity.fields.push(OirField::new(
        "meta",
        OirType::Struct {
            additional: None,
            fields: vec![
                OirField::new("label", OirType::String),
                OirField::new(
                    "geo",
                    OirType::Struct {
                        additional: None,
                        fields: vec![
                            OirField::new("lat", OirType::Number),
                            OirField::new("lng", OirType::Number),
                        ].into(),
                    },
                ),
                OirField::new(
                    "tags",
                    OirType::array(OirType::Struct {
                        additional: None,
                        fields: vec![OirField::new("name", OirType::String)].into(),
                    }),
                ),
            ].into(),
        },
    ));
    entity.fields.push(OirField::new(
        "kind",
        OirType::Union {
            variants: vec![
                OirField::new(
                    "book",
                    OirType::Struct {
                        additional: None,
                        fields: vec![OirField::new("title", OirType::String)].into(),
                    },
                ),
                OirField::new(
                    "movie",
                    OirType::Struct {
                        additional: None,
                        fields: vec![OirField::new("runtime", OirType::Number)].into(),
                    },
                ),
            ].into(),
        },
    ));
    let mut module = OirModule::new("NominalizeDiffApp", "1.0.0");
    module.entities.push(entity);
    module
}

fn entities_ts(module: &OirModule) -> String {
    TypeScriptBackend::new()
        .generate(module)
        .files
        .into_iter()
        .find(|f| f.path == "packages/shared/src/types/entities.ts")
        .expect("entities.ts emitted")
        .content
}

#[test]
fn nominalize_on_hoists_named_interfaces_with_identical_field_content() {
    let base = fixture_module();
    let mut nominal = base.clone();
    let stats = nominalize_module(&mut nominal);
    assert!(stats.registered > 0, "the fixture has anonymous shapes to name");

    let off = entities_ts(&base);
    let on = entities_ts(&nominal);

    // Capture both outputs for the owner diff review.
    let dir = std::path::PathBuf::from("/tmp/orbital-nominalize-ts-diff");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("entities.off.ts"), &off).unwrap();
    std::fs::write(dir.join("entities.on.ts"), &on).unwrap();

    // Flag-OFF output is untouched: no hoisted named interface for the
    // anonymous shapes.
    assert!(
        !off.contains("export type OrderMeta = {"),
        "flag-off must keep the anonymous struct inline:\n{off}"
    );
    // Flag-ON: hoisted named struct aliases with identical field content.
    for (decl, field) in [
        ("export type OrderMeta = {", "label: string;"),
        ("export type OrderMetaGeo = {", "lat: number;"),
        ("export type OrderMetaTagsItem = {", "name: string;"),
    ] {
        assert!(on.contains(decl), "flag-on must hoist {decl}:\n{on}");
        assert!(
            on.contains(field),
            "the hoisted declaration keeps the field content ({field})"
        );
    }
    // The union nominalizes to a hoisted named alias the field references.
    assert!(
        on.contains("export type OrderKind = "),
        "flag-on must hoist the union alias:\n{on}"
    );
    assert!(
        on.contains("kind: OrderKind;") || on.contains("kind?: OrderKind;"),
        "the union field references the alias by bare name:\n{on}"
    );
    // Field content present in BOTH outputs (structural identity). The
    // flag-off inline structs render single-line (`{ lat: number; … }`), so
    // match the field-type pair without the trailing declaration semicolon.
    for content in ["label: string", "lat: number", "lng: number", "name: string"] {
        assert!(off.contains(content), "flag-off keeps {content}:\n{off}");
        assert!(on.contains(content), "flag-on keeps {content}");
    }
}
