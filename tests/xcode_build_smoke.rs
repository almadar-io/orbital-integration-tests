//! E1 native build smoke: compile the notes fixture through `--shell ios`
//! and build the generated Xcode project for real with `orb-build.sh`.
//!
//! Mirrors orbital-shell-python/tests/codegen_py_compile.rs's skip-if-absent
//! shape. Opt-in behind `ORBITAL_NATIVE_SMOKE=1` (shells out to xcodegen +
//! xcodebuild, minutes not seconds) and skips gracefully — never fails —
//! when the flag is unset, the toolchain is missing, or the prebuilt
//! `orbital-ffi` simulator staticlib (`make ios-lib`) isn't there.

#[path = "common/blank_png.rs"]
mod blank_png;

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not navigate to workspace root")
        .to_path_buf()
}

/// Last `n` lines of a string, in original order — for printing a build log
/// tail on failure instead of the whole (often huge) xcodebuild output.
/// `test_<pattern>.1.png` -> `<pattern>`.
fn pattern_of_snapshot(p: &Path) -> String {
    let stem = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    stem.trim_start_matches("test_")
        .trim_end_matches(".1.png")
        .to_owned()
}

fn assert_real_renders(dir: &Path) {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read snapshot dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    blank_png::assert_no_blank_renders(
        &files,
        Some(blank_png::IOS_ZERO_SIZE_ERROR_IMAGE_MD5),
        pattern_of_snapshot,
    );
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[test]
fn xcode_build_smoke() {
    if std::env::var("ORBITAL_NATIVE_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipping xcode_build_smoke: set ORBITAL_NATIVE_SMOKE=1 to run");
        return;
    }

    if Command::new("xcodegen").arg("--version").output().is_err() {
        eprintln!("skipping xcode_build_smoke: xcodegen not on PATH (brew install xcodegen)");
        return;
    }
    if Command::new("xcrun").arg("--version").output().is_err() {
        eprintln!("skipping xcode_build_smoke: xcrun not on PATH (needs Xcode)");
        return;
    }

    let workspace_root = workspace_root();

    let static_lib = workspace_root.join("target/aarch64-apple-ios-sim/release/liborbital_ffi.a");
    if !static_lib.exists() {
        eprintln!(
            "skipping xcode_build_smoke: {} not built — run `make ios-lib` first (this test never builds it)",
            static_lib.display()
        );
        return;
    }

    let fixture = workspace_root.join("test-schemas/std-notes.orb");
    assert!(
        fixture.exists(),
        "fixture not found at {}",
        fixture.display()
    );

    // Skip-if-absent: `orbital-cli` is a sibling crate this test does not
    // build. Prefer a release binary (matches `make release`/`make ios-lib`
    // workflow); fall back to debug.
    // The workspace's own build, never `$ORBITAL_BIN`: the dev profile points that at the
    // installed binary, whose renderers are the last release's, not this crate's (G-ORB-212).
    let binary = {
        let release = workspace_root.join("target/release/orbital");
        let debug = workspace_root.join("target/debug/orbital");
        if release.exists() {
            release
        } else if debug.exists() {
            debug
        } else {
            eprintln!(
                "skipping xcode_build_smoke: orbital CLI not built (run `cargo build --release -p orbital-cli`)"
            );
            return;
        }
    };

    // Unique per run (pid + nanosecond timestamp) — a fixed name raced
    // across concurrent runs (this smoke and, separately, the Android twin's
    // own scratch dir under the same OS temp root). `_cleanup` removes it on
    // any exit path, including a panicking assertion (Drop still runs during
    // unwinding).
    let out_dir = std::env::temp_dir().join(format!(
        "orbital_ios_native_smoke-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&out_dir);
    struct CleanupGuard(PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = CleanupGuard(out_dir.clone());

    let compile = Command::new(&binary)
        .args([
            "compile",
            fixture.to_str().unwrap(),
            "--shell",
            "ios",
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("`orbital compile` failed to launch");
    assert!(
        compile.status.success(),
        "orbital compile --shell ios failed:\nSTDOUT:\n{}\nSTDERR:\n{}",
        tail(&String::from_utf8_lossy(&compile.stdout), 60),
        tail(&String::from_utf8_lossy(&compile.stderr), 60)
    );
    assert_binary_carries_this_crates_renderers(&binary, &out_dir);

    let build_script = out_dir.join("orb-build.sh");
    assert!(
        build_script.exists(),
        "orb-build.sh missing from generated project at {}",
        build_script.display()
    );

    let build = Command::new("./orb-build.sh")
        .current_dir(&out_dir)
        .env("CARGO_BUILD_DIR", workspace_root.join("target"))
        .output()
        .expect("orb-build.sh failed to launch");
    assert!(
        build.status.success(),
        "orb-build.sh (xcodebuild) failed:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
        tail(&String::from_utf8_lossy(&build.stdout), 80),
        tail(&String::from_utf8_lossy(&build.stderr), 80)
    );

    // Locate the built .app the same way orb-build.sh's own --run branch
    // does: the scheme name is project.yml's top-level `name:` line, then
    // ask xcodebuild for its declared BUILT_PRODUCTS_DIR / FULL_PRODUCT_NAME.
    let project_yml = std::fs::read_to_string(out_dir.join("project.yml"))
        .expect("project.yml missing from generated project");
    let app_name = project_yml
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("name:"))
        .map(|s| s.trim().to_string())
        .expect("project.yml missing top-level `name:` line");

    let settings = Command::new("xcodebuild")
        .args([
            "-scheme",
            &app_name,
            "-destination",
            "generic/platform=iOS Simulator",
            "-showBuildSettings",
        ])
        .env("CARGO_BUILD_DIR", workspace_root.join("target"))
        .current_dir(&out_dir)
        .output()
        .expect("xcodebuild -showBuildSettings failed to launch");
    let settings_out = String::from_utf8_lossy(&settings.stdout);

    let built_products_dir = settings_out
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with("BUILT_PRODUCTS_DIR "))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            panic!(
                "BUILT_PRODUCTS_DIR not found in xcodebuild -showBuildSettings output:\n{}",
                tail(&settings_out, 60)
            )
        });
    let full_product_name = settings_out
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with("FULL_PRODUCT_NAME "))
        .and_then(|l| l.split('=').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            panic!(
                "FULL_PRODUCT_NAME not found in xcodebuild -showBuildSettings output:\n{}",
                tail(&settings_out, 60)
            )
        });

    let app_path = PathBuf::from(built_products_dir).join(full_product_name);
    assert!(
        app_path.exists(),
        "built .app not found at {}",
        app_path.display()
    );
    eprintln!("xcode_build_smoke: built {}", app_path.display());

    // Program C wave C4 render gate (docs/Almadar_Compiler_Gaps.md §157):
    // PatternSnapshotTests, generated into Sources/OrbitalAppTests/ by
    // IosBackend, snapshot every non-deferred pattern against the fixtures
    // `almadar-sync pattern-fixtures` embeds. Baselines live checked in at
    // test-schemas/native-conformance/snapshots/ios/<name>.png — copy them
    // into swift-snapshot-testing's default `__Snapshots__/<TestCase>/`
    // location before `xcodebuild test` so a normal (non-recording) run
    // compares against them. `ORBITAL_SNAPSHOT_RECORD=1` maps to
    // `SNAPSHOT_TESTING_RECORD=all` and, on success, copies the freshly
    // recorded images back out to become the new checked-in baseline.
    let snapshot_baseline_dir =
        workspace_root.join("test-schemas/native-conformance/snapshots/ios");
    let generated_snapshot_dir =
        out_dir.join("Sources/OrbitalAppTests/__Snapshots__/PatternSnapshotTests");
    let _ = std::fs::create_dir_all(&generated_snapshot_dir);
    let recording_requested = std::env::var("ORBITAL_SNAPSHOT_RECORD").as_deref() == Ok("1");
    if !recording_requested && snapshot_baseline_dir.exists() {
        for entry in std::fs::read_dir(&snapshot_baseline_dir).expect("read snapshot baseline dir")
        {
            let entry = entry.expect("read snapshot baseline entry");
            let src = entry.path();
            if src.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let dst = generated_snapshot_dir.join(src.file_name().unwrap());
            std::fs::copy(&src, &dst).unwrap_or_else(|e| {
                panic!(
                    "failed to copy baseline snapshot {} -> {}: {e}",
                    src.display(),
                    dst.display()
                )
            });
        }
    }

    let recording = std::env::var("ORBITAL_SNAPSHOT_RECORD").as_deref() == Ok("1");
    if !recording && snapshot_baseline_dir.exists() {
        assert_real_renders(&snapshot_baseline_dir);
    }

    let mut test_cmd = Command::new("./orb-build.sh");
    test_cmd
        .arg("--test")
        .current_dir(&out_dir)
        .env("CARGO_BUILD_DIR", workspace_root.join("target"));
    if recording {
        test_cmd.env("SNAPSHOT_TESTING_RECORD", "all");
    }
    let test_result = test_cmd
        .output()
        .expect("orb-build.sh --test failed to launch");

    // swift-snapshot-testing DELIBERATELY fails every test it records (its
    // own documented behavior — the failure is the signal a human/CI must
    // review the new baseline before trusting it), so a recording run's
    // process exit code is never the pass/fail signal; a normal (compare)
    // run's exit code is.
    if !recording {
        assert!(
            test_result.status.success(),
            "orb-build.sh --test (xcodebuild test) failed:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
            tail(&String::from_utf8_lossy(&test_result.stdout), 120),
            tail(&String::from_utf8_lossy(&test_result.stderr), 120)
        );
        eprintln!("xcode_build_smoke: PatternSnapshotTests passed");
        return;
    }

    let recorded: Vec<_> = std::fs::read_dir(&generated_snapshot_dir)
        .expect("read generated snapshot dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    assert!(
        !recorded.is_empty(),
        "recording run produced no PNGs at {} — xcodebuild test tail:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
        generated_snapshot_dir.display(),
        tail(&String::from_utf8_lossy(&test_result.stdout), 120),
        tail(&String::from_utf8_lossy(&test_result.stderr), 120)
    );

    let fixture_dir = workspace_root.join("test-schemas/native-conformance/pattern-fixtures");
    let mut missing: Vec<String> = std::fs::read_dir(&fixture_dir)
        .expect("read pattern fixture dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .filter(|name| !orbital_shell_ios::ios_snapshot_excluded(name))
        .filter(|name| {
            let expected = format!("test_{}.1.png", name.replace('-', "_"));
            !recorded
                .iter()
                .any(|r| r.file_name().and_then(|s| s.to_str()) == Some(expected.as_str()))
        })
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "recording run produced no PNG for {} fixture(s): {}",
        missing.len(),
        missing.join(", ")
    );

    blank_png::assert_no_blank_renders(
        &recorded,
        Some(blank_png::IOS_ZERO_SIZE_ERROR_IMAGE_MD5),
        pattern_of_snapshot,
    );

    std::fs::create_dir_all(&snapshot_baseline_dir).expect("create snapshot baseline dir");
    for src in &recorded {
        let dst = snapshot_baseline_dir.join(src.file_name().unwrap());
        std::fs::copy(src, &dst).unwrap_or_else(|e| {
            panic!(
                "failed to record baseline snapshot {} -> {}: {e}",
                src.display(),
                dst.display()
            )
        });
    }
    eprintln!(
        "xcode_build_smoke: recorded {} baseline(s) to {}",
        recorded.len(),
        snapshot_baseline_dir.display()
    );
}

/// The generated project's `CorePatternRegistry` is byte-for-byte the installed pack's: a binary built
/// before the renderers changed would record or compare stale snapshots.
fn assert_binary_carries_this_crates_renderers(binary: &Path, out_dir: &Path) {
    let pack = orbital_shell_ios::require_renderer_pack().expect("installed renderer pack");
    let source = Path::new("CorePatternRegistry.swift");
    let expected = pack
        .source("CorePatternRegistry.swift")
        .expect("the pack's CorePatternRegistry")
        .as_bytes()
        .to_vec();
    let generated = find_file(out_dir, source.file_name().unwrap()).unwrap_or_else(|| {
        panic!(
            "no {} in the generated project",
            source.file_name().unwrap().to_string_lossy()
        )
    });
    assert!(
        std::fs::read(&generated).expect("generated CorePatternRegistry") == expected,
        "{} was built before this crate's renderers changed (its CorePatternRegistry differs): \
         rebuild it with `cargo build --release -p orbital-cli`",
        binary.display()
    );
}

fn find_file(dir: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name() == Some(name) {
            return Some(path);
        }
    }
    None
}
