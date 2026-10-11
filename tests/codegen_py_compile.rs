//! P3 Python shell codegen smoke: emit + py_compile gate.
//!
//! Mirrors orbital-shell-typescript/tests/codegen_typecheck.rs.
//! The full procedure:
//!   1. Feed the ML fixture `.orb` through the compiler's Python backend.
//!   2. Write every emitted `.py` file to a temp directory.
//!   3. Run `python3 -m compileall <dir>` (exit 0 = no syntax errors).
//!
//! This is an in-process test — it does NOT require FastAPI or PyTorch to be
//! installed; `py_compile` only checks syntax, not runtime imports. The
//! `--ignored` flag is kept consistent with the TS shell peer for the same
//! reason: CI runs the procedure end-to-end via the orbital-verify gate, while
//! the fast Rust unit-test loop keeps it opt-in.

use std::path::PathBuf;
use std::process::Command;

/// Emit the Python ML fixture through the compiler and syntax-check with
/// `python3 -m compileall`.
///
/// Runs by default; skips gracefully (not a failure) when `python3` is absent,
/// matching the `orb verify --shell python` gate's skip-if-absent behavior.
#[test]
fn codegen_py_compile_smoke() {
    // Skip-if-absent precondition: `python3` must be on PATH. In environments
    // without it (e.g. minimal CI images) this test no-ops rather than failing.
    match Command::new("python3").args(["--version"]).output() {
        Ok(v) if v.status.success() => {}
        _ => {
            eprintln!("skipping codegen_py_compile_smoke: python3 not available on PATH");
            return;
        }
    }

    // Locate the fixture .orb (relative to the workspace root, which is 3
    // levels above CARGO_MANIFEST_DIR = crates/orbital-integration-tests/).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not navigate to workspace root");
    let fixture = workspace_root.join("test-schemas/Orbital_Pattern_Python.orb");
    assert!(
        fixture.exists(),
        "ML fixture not found at {}",
        fixture.display()
    );

    // Compile via the CLI binary.
    let out_dir = std::env::temp_dir().join("orbital_py_codegen_test");
    let _ = std::fs::remove_dir_all(&out_dir);

    // Skip-if-absent: the `orbital` CLI binary is built by a different crate
    // (orbital-cli); `cargo test -p orbital-integration-tests` does not build it. If
    // it hasn't been built yet, no-op rather than failing with a confusing
    // launch error (run `cargo build -p orbital-cli` first to exercise this).
    let binary = workspace_root.join("target/debug/orbital");
    if !binary.exists() {
        eprintln!(
            "skipping codegen_py_compile_smoke: {} not built (run `cargo build -p orbital-cli`)",
            binary.display()
        );
        return;
    }
    let status = Command::new(binary)
        .args([
            "compile",
            fixture.to_str().unwrap(),
            "--shell",
            "python",
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .status()
        .expect("`orbital compile` failed to launch");
    assert!(
        status.success(),
        "`orbital compile` exited non-zero for the ML fixture"
    );

    // py_compile all emitted files.
    let result = Command::new("python3")
        .args(["-m", "compileall", out_dir.to_str().unwrap()])
        .output()
        .expect("python3 -m compileall failed to launch");
    assert!(
        result.status.success(),
        "py_compile found syntax errors in emitted Python:\n{}",
        String::from_utf8_lossy(&result.stdout)
    );

    // Adversarial: inject a syntax error into one emitted file and confirm
    // `python3 -m py_compile` on THAT FILE returns non-zero.
    // `compileall` on a directory continues past errors (exit 0 even with
    // SyntaxErrors in Python 3.12+), so we probe the individual file directly.
    let trait_file = out_dir.join(
        "src/orbital_app/generated/traits/neural_network_manager.py",
    );
    if trait_file.exists() {
        let original = std::fs::read_to_string(&trait_file).expect("read trait file");
        // Append a bare colon — always a SyntaxError.
        let broken = format!("{}\n:\n", original);
        std::fs::write(&trait_file, broken).expect("write broken file");

        let broken_result = Command::new("python3")
            .args(["-m", "py_compile", trait_file.to_str().unwrap()])
            .output()
            .expect("python3 -m py_compile failed to launch");
        assert!(
            !broken_result.status.success(),
            "py_compile should have failed on the deliberately broken file but did not"
        );

        // Restore original so subsequent steps see clean files.
        std::fs::write(&trait_file, original).expect("restore trait file");
    }

    // Import smoke: verify the generated package imports cleanly at the module
    // level. The generated code imports its runtime deps (pydantic/fastapi/torch)
    // at top level, so on a minimal CI image without them the import raises
    // `ModuleNotFoundError: No module named '<dep>'`. When the missing module is
    // a THIRD-PARTY dep, that's a skip (env gap) — the py_compile syntax gate
    // above already ran. When the missing module is our own `orbital_app…`
    // package, that's a real codegen bug (bad relative import) and must fail.
    // (The traceback always mentions the orbital_app file paths, so we key off
    // the `No module named '…'` module name, not a substring of the whole log.)
    let src_dir = out_dir.join("src");
    let smoke = Command::new("python3")
        .args([
            "-c",
            "import orbital_app.generated.entities; \
             import orbital_app.generated.traits; \
             import orbital_app.generated.routes; \
             print('import smoke: OK')",
        ])
        .env("PYTHONPATH", src_dir.to_str().unwrap())
        .output()
        .expect("python3 import smoke failed to launch");
    if !smoke.status.success() {
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&smoke.stdout),
            String::from_utf8_lossy(&smoke.stderr)
        );
        let missing_our_package = output.contains("No module named 'orbital_app");
        if output.contains("ModuleNotFoundError") && !missing_our_package {
            eprintln!(
                "skipping import smoke: generated-code runtime deps not installed:\n{}",
                output
            );
            return;
        }
        panic!("import smoke failed:\n{}", output);
    }
    let smoke_out = String::from_utf8_lossy(&smoke.stdout);
    assert!(
        smoke_out.contains("import smoke: OK"),
        "unexpected import smoke output: {}",
        smoke_out
    );
}

/// Verify that `python3` is discoverable when this test is run explicitly.
#[test]
#[ignore = "P3 precondition: confirms python3 is on PATH; run with --include-ignored."]
fn python3_on_path() {
    let out = Command::new("python3")
        .arg("--version")
        .output()
        .expect("python3 not found on PATH");
    assert!(
        out.status.success(),
        "`python3 --version` exited non-zero: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}
