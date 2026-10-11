use std::path::Path;

// Both web templates' client builds run route-documents.mjs; each ships its own
// copy (the template is copied whole into the generated app). They must stay one script.
#[test]
fn hono_template_carries_the_express_route_documents_script() {
    let templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript/templates");
    let read = |shell: &str| {
        let path = templates.join(shell).join("packages/client/scripts/route-documents.mjs");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    };
    assert_eq!(read("almadar-shell-hono"), read("almadar-shell"));
}
