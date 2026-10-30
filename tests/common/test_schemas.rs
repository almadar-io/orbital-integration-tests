//! Every test-schema a test loads is validated first, through the compiler's own validate API, so a schema that
//! drifts from the language is a named failure here and not a confusing one downstream.

use std::path::Path;

/// Schemas that still fail validation, by file name and the reason. Each needs a fix in std, outside this repo; none is
/// skipped silently.
pub const NOT_VALID: &[(&str, &str)] = &[
    ("std-notes.orb", "ORB_X_OPERATOR_NULL_OPERAND 47, ORB_X_OPERATOR_ARG_UNDETERMINED 25, ORB_X_OPERATOR_ARG_TYPE_MISMATCH 7, ORB_X_SET_RETURN_TYPE_MISMATCH 3, all inside referenced std atoms the std lanes are fixing"),
    ("rtl-source.orb", "ORB_E_INVALID_FIELD_NAME: the std entity-shape lint decides `latin` from raw text and an .orb carries no source language, so the Arabic entity name is checked as English"),
];

pub fn not_valid_reason(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    NOT_VALID.iter().find(|(file, _)| *file == name).map(|(_, reason)| *reason)
}

/// Resolve `path` as `orb compile` does and assert comprehensive validation finds no error and no warning.
pub fn assert_valid(path: &Path, options: &orbital_compiler::CompilerOptions) {
    if not_valid_reason(path).is_some() {
        return;
    }
    let schema = orbital_compiler::phases::parse_file(path).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
    let resolved = orbital_compiler::phases::resolve_compiled_program_with_path(schema, options, Some(path))
        .unwrap_or_else(|e| panic!("{} does not resolve: {e}", path.display()));
    let result = orbital_compiler::phases::validation::validate_comprehensive_without_services(&resolved);
    let findings: Vec<String> = result.errors.iter().chain(result.warnings.iter()).map(|f| format!("{f:?}")).collect();
    assert!(findings.is_empty(), "{} fails validation:\n{}", path.display(), findings.join("\n"));
}
