//! `strict: true` already implies `strictNullChecks: true`; an explicit
//! `"strictNullChecks": false` in an emitted tsconfig silently turns off the one
//! check that catches `Entity | null` misuse. No shell template may carry it.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn template_roots() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("read crates/").flatten() {
        for dir in ["templates"] {
            let template = entry.path().join(dir);
            if template.is_dir() {
                roots.push(template);
            }
        }
    }
    roots
}

fn collect_tsconfig_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "node_modules" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_tsconfig_files(&path, out);
        } else if name == "tsconfig.json" {
            out.push(path);
        }
    }
}

#[test]
fn no_template_tsconfig_disables_strict_null_checks() {
    let mut tsconfigs = Vec::new();
    for root in template_roots() {
        collect_tsconfig_files(&root, &mut tsconfigs);
    }
    assert!(!tsconfigs.is_empty(), "found no tsconfig.json under any template");
    let violations: Vec<String> = tsconfigs
        .iter()
        .filter(|path| {
            let content = std::fs::read_to_string(path).expect("read tsconfig.json");
            content.contains("\"strictNullChecks\": false") || content.contains("\"strictNullChecks\":false")
        })
        .map(|path| path.display().to_string())
        .collect();
    assert!(violations.is_empty(), "strictNullChecks: false found in template tsconfig(s): {violations:?}");
}
