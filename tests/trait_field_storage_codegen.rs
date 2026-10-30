//! A `trait`-typed entity field (language trio) holds a local embed `@trait.X`,
//! a stored trait value `{ behavior, trait, …overrides }`, or `null` (unbound):
//! the shared types and zod schemas inline that storage shape. A config-only
//! `trait` knob keeps the local-embed string form.

use orbital_compiler::phases::resolve_to_oir;
use orbital_core::generate::Backend;
use orbital_core::schema::OrbitalSchema;
use orbital_shell_typescript::TypeScriptBackend;

fn schema(entity_fields: serde_json::Value) -> OrbitalSchema {
    serde_json::from_value(serde_json::json!({
        "name": "PickerApp",
        "version": "1.0.0",
        "orbitals": [{
            "name": "PickerOrbital",
            "entity": { "name": "Picker", "persistence": "runtime", "fields": entity_fields },
            "traits": [],
            "pages": []
        }]
    }))
    .expect("fixture schema must deserialize")
}

fn file(schema: &OrbitalSchema, path: &str) -> String {
    let module = resolve_to_oir(schema).expect("schema resolves to OIR");
    TypeScriptBackend::new()
        .generate(&module)
        .files
        .into_iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} emitted"))
        .content
}

fn picker_with_widget() -> OrbitalSchema {
    schema(serde_json::json!([
        { "name": "id", "type": "string", "required": true },
        { "name": "widget", "type": "trait" }
    ]))
}

#[test]
fn a_trait_entity_field_is_typed_as_embed_stored_value_or_null() {
    let types = file(
        &picker_with_widget(),
        "packages/shared/src/types/entities.ts",
    );
    assert!(
        types.contains("type TraitFieldRef = `@trait.${string}` | TraitFieldStored | null;"),
        "{types}"
    );
    assert!(types.contains("interface TraitFieldStored {"), "{types}");
    assert!(types.contains("widget?: TraitFieldRef | null;"), "{types}");
}

#[test]
fn a_trait_entity_field_validates_through_the_inlined_schema() {
    let schemas = file(
        &picker_with_widget(),
        "packages/shared/src/schemas/entities.ts",
    );
    assert!(
        schemas.contains("widget: TraitFieldValueSchema"),
        "{schemas}"
    );
    assert!(
        schemas.contains("const TraitFieldValueSchema: z.ZodType<TraitFieldRef> = z.union(["),
        "{schemas}"
    );
    assert!(schemas.contains("FieldValueSchema"), "{schemas}");
    assert!(
        schemas.contains("import type { FieldValue } from '@almadar/core';"),
        "{schemas}"
    );
    assert!(
        !schemas.contains("regex(/^@trait"),
        "the old string-only regex must not validate a trait field:\n{schemas}"
    );
}

#[test]
fn control_an_entity_without_trait_fields_inlines_neither() {
    let plain = schema(serde_json::json!([
        { "name": "id", "type": "string", "required": true },
        { "name": "title", "type": "string" }
    ]));
    let types = file(&plain, "packages/shared/src/types/entities.ts");
    let schemas = file(&plain, "packages/shared/src/schemas/entities.ts");
    assert!(!types.contains("TraitFieldStored"), "{types}");
    assert!(!schemas.contains("TraitFieldValueSchema"), "{schemas}");
}
