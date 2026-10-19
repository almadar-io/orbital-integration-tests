//! A shell program must write exactly the app the in-process shell wrote:
//! the same compile, once through the linked TypeScript shell (today's
//! `orb compile`) and once through `TypeScriptProgram` with its request
//! crossing the wire as JSON.

use orbital_compiler::generate::{generate_from_file_with_source, generate_from_file_with_source_using, GenerateError};
use orbital_compiler::GenerateOptions;
use orbital_core::shell_protocol::{handle, GenerateRequest, ShellProgram, ShellReply, IR_VERSION};
use orbital_shell_typescript::shell_program::TypeScriptProgram;
use orbital_shell_typescript::{express_shell, TypeScriptBackend};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const STALE: &[&str] = &["08-game.orb"];

fn programs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-schemas");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("orb"))
        .filter(|p| !STALE.iter().any(|s| p.file_name().is_some_and(|f| f == *s)))
        .collect();
    out.sort();
    out
}

fn options() -> GenerateOptions {
    orbital_compiler::loader::register_source_lowerer(orbital_test_support::lower_lolo_file);
    let mut compiler = orbital_compiler::CompilerOptions::from_env();
    let start = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-schemas");
    let user_store = orbital_core::behavior_packages::user_store_dir().unwrap_or_else(|| start.clone());
    compiler.inline.loader_config.packages =
        orbital_core::behavior_packages::discover(&start, &user_store).expect("behavior package discovery");
    GenerateOptions { compiler, copy_template: true, ..Default::default() }
}

fn resolved(path: &Path, options: &GenerateOptions) -> orbital_core::OrbitalSchema {
    let schema = orbital_compiler::phases::parse_file(path).expect("parse");
    orbital_compiler::phases::resolve_compiled_program_with_path(schema, &options.compiler, Some(path)).expect("resolve")
}

/// Today's linked `orb compile` branch for `shell`, verbatim.
fn in_process(shell: &str, path: &Path, out: &Path) -> Result<(), String> {
    let options = options();
    let e = |e: GenerateError| e.to_string();
    match shell {
        "typescript" => {
            let backend = TypeScriptBackend { browser_schema: Some(resolved(path, &options)), ..TypeScriptBackend::default() };
            generate_from_file_with_source(path, path, out, &express_shell(), &backend, &options).map_err(e)?;
            orbital_shell_typescript::shell_program::copy_locales(path, out).expect("copy locales");
            orbital_shell_typescript::shell_program::add_dependencies(&out.join("packages/server/package.json"), orbital_shell_typescript::KERNEL_SERVER_DEPENDENCIES)
                .expect("add kernel dependencies");
        }
        "python" => {
            generate_from_file_with_source(path, path, out, &orbital_shell_python::PythonShell::new(), &orbital_shell_python::PythonBackend::new(), &options).map_err(e)?;
        }
        "rust" => {
            generate_from_file_with_source(path, path, out, &orbital_shell_rust::RustShell::new(), &orbital_shell_rust::RustBackend::new(), &options).map_err(e)?;
        }
        "android" => {
            orbital_shell_android::require_renderer_pack().map_err(|err| err.to_string())?;
            let backend = orbital_shell_android::AndroidBackend::new().with_resolved_schema(resolved(path, &options));
            generate_from_file_with_source(path, path, out, &orbital_shell_android::AndroidShell::new(), &backend, &options).map_err(e)?;
        }
        "ios" => {
            use orbital_core::shell::Shell as _;
            orbital_shell_ios::require_renderer_pack().map_err(|err| err.to_string())?;
            let shell = orbital_shell_ios::IosShell::new();
            let backend = orbital_shell_ios::IosBackend::new().with_resolved_schema(resolved(path, &options));
            let result = generate_from_file_with_source(path, path, out, &shell, &backend, &options).map_err(e)?;
            shell.validate_structure(out).map_err(|err| err.to_string())?;
            let manifest: Vec<String> = result.files.iter().map(|f| f.path.clone()).collect();
            shell.validate_generated_layout(out, &manifest).map_err(|err| err.to_string())?;
        }
        "chrome" => {
            let resolved = resolved(path, &options);
            let runtimes = options.compiler.service_runtimes.clone().unwrap_or_else(|| orbital_core::language::registry::baked_service_registry().runtimes.clone());
            let program = orbital_shell_chrome::check_program(&serde_json::to_value(&resolved).unwrap(), &runtimes)
                .map_err(|errors| errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))?;
            let backend = orbital_shell_chrome::ChromeBackend::new(resolved, program);
            generate_from_file_with_source(path, path, out, &orbital_shell_chrome::ChromeShell::new(), &backend, &options).map_err(e)?;
        }
        other => panic!("no linked branch for {other}"),
    }
    Ok(())
}

fn program(shell: &str) -> Box<dyn ShellProgram> {
    match shell {
        "typescript" => Box::new(TypeScriptProgram),
        "python" => Box::new(orbital_shell_python::shell_program::PythonProgram),
        "rust" => Box::new(orbital_shell_rust::shell_program::RustProgram),
        "android" => Box::new(orbital_shell_android::shell_program::AndroidProgram),
        "ios" => Box::new(orbital_shell_ios::shell_program::IosProgram),
        "chrome" => Box::new(orbital_shell_chrome::shell_program::ChromeProgram),
        other => panic!("no program for {other}"),
    }
}

const SHELLS: &[&str] = &["typescript", "python", "rust", "android", "ios", "chrome"];

fn through_program(shell: &str, path: &Path, out: &Path) -> Result<(), String> {
    let options = options();
    let program = program(shell);
    let resolved = program.describe().needs_resolved.then(|| resolved(path, &options));
    generate_from_file_with_source_using(path, path, out, &options, |module, out| {
        let request = GenerateRequest {
            ir_version: IR_VERSION,
            module: module.clone(),
            resolved: resolved.clone(),
            source_path: Some(path.to_path_buf()),
            write: orbital_core::project::WriteOptions {
                copy_template: options.copy_template,
                clean: options.clean,
                verbose: false,
                mode: options.mode,
                service_runtimes: options.compiler.service_runtimes.clone(),
            },
            options: BTreeMap::new(),
        };
        let wire: GenerateRequest = serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        match handle(program.as_ref(), &wire, out) {
            ShellReply::Ok(response) => Ok(response),
            ShellReply::Error(failure) => Err(GenerateError::Backend(failure.message)),
        }
    })
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
            if rel == "resolved.orb" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

#[test]
fn every_shell_program_writes_the_app_its_linked_shell_wrote() {
    let base = std::env::temp_dir().join(format!("orb-shell-parity-{}", std::process::id()));
    let mut mismatches = Vec::new();
    let mut built = 0;
    for shell in SHELLS {
    for path in programs() {
        let name = format!("{shell}/{}", path.file_stem().unwrap().to_string_lossy());
        let (a, b) = (base.join(&name).join("linked/app"), base.join(&name).join("program/app"));
        match (in_process(shell, &path, &a), through_program(shell, &path, &b)) {
            (Ok(()), Ok(())) => {}
            (Err(linked), Err(program)) => {
                // The program reports the backend's own message, wrapped once more by its failure code.
                if !program.contains(linked.trim_start_matches("Backend error: ")) {
                    mismatches.push(format!("{name}: different refusals:\n  linked: {linked}\n  program: {program}"));
                }
                continue;
            }
            (linked, program) => {
                mismatches.push(format!("{name}: linked {linked:?} but program {program:?}"));
                continue;
            }
        }
        let (ta, tb) = (tree(&a), tree(&b));
        let keys: BTreeSet<&String> = ta.keys().chain(tb.keys()).collect();
        let differing: Vec<&String> = keys.into_iter().filter(|k| ta.get(*k) != tb.get(*k)).collect();
        if !differing.is_empty() {
            mismatches.push(format!("{name}: {differing:?}"));
        }
        built += 1;
        if *shell == "typescript" {
            assert!(tb.contains_key("packages/client/scripts/route-documents.mjs"), "{name}: the web template's route-documents script is missing");
        }
    }
    }
    if mismatches.is_empty() { let _ = std::fs::remove_dir_all(&base); }
    assert!(built > SHELLS.len(), "parity compared only {built} apps");
    assert!(mismatches.is_empty(), "the program wrote a different app:\n{}", mismatches.join("\n"));
}

#[test]
fn control_the_program_describes_the_typescript_shell() {
    let description = TypeScriptProgram.describe();
    assert_eq!(description.name, "typescript");
    assert!(description.ir_versions.accepts(IR_VERSION));
    assert!(description.needs_resolved);
}
