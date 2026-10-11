use orbital_compiler::phases::validation::{entity::validate_field, SchemaLanguage};
use orbital_core::schema::FieldDefinition;

// The TS shell's `persist_target_intrinsics_escape_valid_localized_field_names`
// pins the codegen escaping of these same names.
#[test]
fn localized_intrinsic_field_names_validate() {
    for name in ["draft", "حقل'\\مؤقت"] {
        let field: FieldDefinition = serde_json::from_value(serde_json::json!({
            "name": name, "type": "string", "intrinsic": true
        }))
        .unwrap();
        let validation = validate_field(
            &field,
            "entity.fields[0]",
            &Default::default(),
            &mut Default::default(),
            SchemaLanguage::NonLatin("ar".into()).should_check_ascii_naming(),
        );
        assert!(validation.errors.is_empty(), "{name}: {:?}", validation.errors);
        assert!(validation.warnings.is_empty(), "{name}: {:?}", validation.warnings);
    }
}
