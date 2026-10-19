//! A shell program receives the resolved IR as JSON. Every shell must generate
//! exactly the same files from the IR that crossed the wire as from the IR the
//! compiler produced in-process; anything the wire drops changes an app.

use orbital_core::generate::{Backend, GeneratedOutput, ShellMode};
use orbital_core::ir::OirModule;
use std::path::{Path, PathBuf};

/// Programs that no longer load, by name and reason — never skipped silently.
const STALE: &[(&str, &str)] = &[("08-game.orb", "uses std-gameflow, which std no longer has")];

fn orb_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("orb"))
        .collect();
    out.sort();
    out
}

fn programs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-schemas");
    let mut out = orb_files(&root);
    out.extend(orb_files(&root.join("native-conformance/features")));
    out.retain(|p| !STALE.iter().any(|(name, _)| p.file_name().is_some_and(|f| f == *name)));
    out
}

/// Compiled as `orb compile` would from the program's directory: the installed
/// behavior packages discovered from there, the `.lolo` lowerer registered.
fn compile(path: &Path) -> OirModule {
    orbital_compiler::loader::register_source_lowerer(orbital_test_support::lower_lolo_file);
    let start = path.parent().expect("program dir").to_path_buf();
    let user_store = orbital_core::behavior_packages::user_store_dir().unwrap_or_else(|| start.clone());
    let packages = orbital_core::behavior_packages::discover(&start, &user_store).expect("behavior package discovery");
    let options = orbital_compiler::CompilerOptions {
        inline: orbital_compiler::phases::inline::InlineConfig {
            loader_config: orbital_compiler::loader::LoaderConfig { packages, ..Default::default() },
            ..Default::default()
        },
        ..Default::default()
    };
    orbital_compiler::phases::compile_file(path, &options)
        .unwrap_or_else(|e| panic!("{} does not compile: {e}", path.display()))
}

fn over_the_wire(module: &OirModule) -> OirModule {
    serde_json::from_str(&serde_json::to_string(module).expect("serialize IR")).expect("deserialize IR")
}

fn backends() -> Vec<(&'static str, Box<dyn Backend>)> {
    orbital_shell_ios::require_renderer_pack().expect("ios renderer pack");
    orbital_shell_android::require_renderer_pack().expect("android renderer pack");
    vec![
        ("typescript", Box::new(orbital_shell_typescript::TypeScriptBackend::new())),
        ("python", Box::new(orbital_shell_python::PythonBackend::new())),
        ("rust", Box::new(orbital_shell_rust::RustBackend::new())),
        ("ios", Box::new(orbital_shell_ios::IosBackend::new())),
        ("android", Box::new(orbital_shell_android::AndroidBackend::new())),
    ]
}

fn files(output: &GeneratedOutput) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = output.files.iter().map(|f| (f.path.clone(), f.content.clone())).collect();
    files.sort();
    files
}

#[test]
fn every_shell_generates_the_same_files_from_the_wire_ir() {
    let mut mismatches = Vec::new();
    for path in programs() {
        let module = compile(&path);
        let wire = over_the_wire(&module);
        for (shell, backend) in backends() {
            let direct = files(&backend.generate_with_mode(&module, ShellMode::Full));
            let crossed = files(&backend.generate_with_mode(&wire, ShellMode::Full));
            if direct != crossed {
                let differing: Vec<&str> = direct
                    .iter()
                    .zip(crossed.iter())
                    .filter(|(a, b)| a != b)
                    .map(|(a, _)| a.0.as_str())
                    .collect();
                mismatches.push(format!(
                    "{} via {shell}: {} vs {} files; differing: {differing:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    direct.len(),
                    crossed.len()
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "the IR wire loses something:\n{}", mismatches.join("\n"));
}

#[test]
fn control_the_wire_round_trip_is_stable() {
    for path in programs() {
        let module = compile(&path);
        let once = serde_json::to_value(over_the_wire(&module)).unwrap();
        let twice = serde_json::to_value(over_the_wire(&over_the_wire(&module))).unwrap();
        assert_eq!(once, twice, "{}", path.display());
    }
}

#[test]
fn every_shell_generates_the_same_files_twice_from_one_ir() {
    let mut unstable = Vec::new();
    for path in programs() {
        let module = compile(&path);
        for ((shell, first), (_, second)) in backends().into_iter().zip(backends()) {
            let a = files(&first.generate_with_mode(&module, ShellMode::Full));
            let b = files(&second.generate_with_mode(&module, ShellMode::Full));
            for ((pa, ca), (_, cb)) in a.iter().zip(b.iter()) {
                if ca != cb {
                    let line = ca.lines().zip(cb.lines()).find(|(x, y)| x != y).map(|(x, _)| x.trim().to_string()).unwrap_or_default();
                    unstable.push(format!("{} via {shell}: {pa} — first differing line: {line}", path.file_name().unwrap().to_string_lossy()));
                }
            }
        }
    }
    unstable.sort();
    unstable.dedup();
    assert!(unstable.is_empty(), "codegen is not deterministic:\n{}", unstable.join("\n"));
}
