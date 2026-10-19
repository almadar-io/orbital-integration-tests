//! The shell templates install hoisted (`node-linker=hoisted`), so a dependency that ships its
//! own `tsc` (`typescript@7` brings `@typescript/old`, TS 6) can take the app root's
//! `node_modules/.bin/tsc`, which every package's bare `tsc --noEmit` then runs (G-CROSS-143).
//! The root declares the TypeScript its packages typecheck with, so the root bin is that one.

use std::path::{Path, PathBuf};

fn templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../orbital-shell-typescript/templates")
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn each_shell_root_declares_the_typescript_its_packages_typecheck_with() {
    for shell in ["almadar-shell"] {
        let root = templates_dir().join(shell);
        let root_ts = json(&root.join("package.json"))["devDependencies"]["typescript"].clone();
        let mut typechecking = 0;
        for entry in std::fs::read_dir(root.join("packages")).unwrap() {
            let manifest = entry.unwrap().path().join("package.json");
            if !manifest.exists() {
                continue;
            }
            let pkg = json(&manifest);
            let Some(script) = pkg["scripts"]["typecheck"].as_str() else { continue };
            if !script.starts_with("tsc") {
                continue;
            }
            typechecking += 1;
            let declared = pkg["devDependencies"]["typescript"].clone();
            assert!(declared.is_string(), "{} runs tsc without declaring typescript", manifest.display());
            assert_eq!(root_ts, declared, "{shell}: the root bin must be the compiler {} declares", manifest.display());
        }
        assert!(typechecking > 0, "control: {shell} has packages that typecheck with tsc");
    }
}
