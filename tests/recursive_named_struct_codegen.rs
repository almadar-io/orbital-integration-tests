//! Recursive named struct types (feature test 1 + spec §4): emitting a
//! fixture with `type MenuItem = { label : string!, subMenu : [MenuItem] }`
//! on an entity field must produce a real, HOISTED, self-referential TS
//! interface (`interface MenuItem { subMenu: MenuItem[] }`), not the
//! `opaque`/`Json` degradation the recursive back-reference used to fall
//! into — plus the matching `z.lazy()` Zod schema.
//!
//! The fixture is authored directly as `.orb` wire JSON (the shape
//! `lower/entity.rs`'s RESOLVING cycle guard actually produces: a `union`
//! FieldType carrying `values: ["MenuItem"]` and no `properties` at the
//! recursion point) rather than round-tripped through `.lolo`, so this test
//! stays in `orbital-shell-typescript` without a new cross-crate dependency
//! on `orbital-lolo`; the lowering side is pinned separately in
//! `orbital-lolo`'s own test suite.

use orbital_compiler::phases::resolve_to_oir;
use orbital_core::generate::Backend;
use orbital_core::schema::OrbitalSchema;
use orbital_shell_typescript::TypeScriptBackend;

fn menu_schema() -> OrbitalSchema {
    serde_json::from_value(serde_json::json!({
        "name": "MenuApp",
        "version": "1.0.0",
        "orbitals": [
            {
                "name": "MenuOrbital",
                "entity": {
                    "name": "Menu",
                    "persistence": "persistent",
                    "collection": "menus",
                    "fields": [
                        { "name": "id", "type": "string", "required": true },
                        {
                            "name": "root",
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "required": true },
                                "subMenu": {
                                    "type": "array",
                                    "items": { "type": "union", "values": ["MenuItem"] }
                                }
                            }
                        }
                    ]
                },
                "traits": [],
                "pages": []
            }
        ]
    }))
    .expect("fixture schema must deserialize")
}

/// A named struct is a type alias, not an interface: an interface has no implicit index
/// signature, so a payload or entity field holding one would not fit `EventPayloadValue`
/// (TS2411/TS2352 on the studio generator's `KnobField`, G-ORB-150).
#[test]
fn recursive_struct_hoists_a_named_ts_interface() {
    let schema = menu_schema();
    let module = resolve_to_oir(&schema).expect("schema resolves to OIR");
    let output = TypeScriptBackend::new().generate(&module);
    let entities_ts = output
        .files
        .iter()
        .find(|f| f.path == "packages/shared/src/types/entities.ts")
        .expect("entities.ts emitted")
        .content
        .clone();

    assert!(
        entities_ts.contains("export type MenuItem = {"),
        "must hoist a named MenuItem type alias:\n{entities_ts}"
    );
    assert!(
        entities_ts.contains("subMenu: MenuItem[];") || entities_ts.contains("subMenu?: MenuItem[] | null }"),
        "subMenu must reference MenuItem recursively, not degrade to Json/opaque:\n{entities_ts}"
    );
    assert!(
        !entities_ts.contains("Record<string, FieldValue"),
        "the recursive field must not have degraded to the Json fallback shape:\n{entities_ts}"
    );

    // Entities lead the file; named types are hoisted after them. TS type
    // position forward-references are legal either way, but the sourcemap
    // anchor (first_ts_entity_name) must see an entity interface first, so
    // the spec's hoisting is "after entities" — this pins that placement.
    let menu_item_pos = entities_ts.find("export type MenuItem = {").unwrap();
    let menu_entity_pos = entities_ts
        .find("export interface Menu extends EntityRow {")
        .expect("Menu entity interface emitted");
    assert!(
        menu_item_pos > menu_entity_pos,
        "MenuItem must be hoisted after the entity interfaces (entities lead entities.ts)"
    );
}

#[test]
fn recursive_struct_hoists_a_lazy_zod_schema() {
    let schema = menu_schema();
    let module = resolve_to_oir(&schema).expect("schema resolves to OIR");
    let output = TypeScriptBackend::new().generate(&module);
    let schemas_ts = output
        .files
        .iter()
        .find(|f| f.path == "packages/shared/src/schemas/entities.ts")
        .expect("entities schema emitted")
        .content
        .clone();

    assert!(
        schemas_ts.contains("MenuItemSchema: z.ZodType<MenuItem>"),
        "must hoist a typed, named MenuItemSchema:\n{schemas_ts}"
    );
    assert!(
        schemas_ts.contains("z.lazy(() => MenuItemSchema)"),
        "the self-reference must use the standard z.lazy() recursive-schema idiom:\n{schemas_ts}"
    );
}
