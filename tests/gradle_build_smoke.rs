//! E1 native build smoke: compile the notes fixture through `--shell android`
//! and typecheck the generated Gradle project for real with
//! `./gradlew assembleDebug`.
//!
//! Mirrors orbital-shell-python/tests/codegen_py_compile.rs's skip-if-absent
//! shape. Opt-in behind `ORBITAL_NATIVE_SMOKE=1` (shells out to Gradle,
//! minutes not seconds) and skips gracefully — never fails — when the flag
//! is unset, the Android SDK isn't configured, or the prebuilt
//! `orbital-ffi` `.so` (`make android-lib`) isn't there. This test
//! never builds the native lib itself.

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
/// tail on failure instead of the whole (often huge) Gradle output.
/// `<pattern>.png` -> `<pattern>`.
fn pattern_of_snapshot(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// A per-run temp dir (pid + nanos, never reused) so two concurrent
/// `gradle_build_smoke` runs (e.g. this wave's iOS/Android agents) never race
/// on the same `orbital_android_native_smoke` path. Removed on drop —
/// including on a panicking assert, since the harness unwinds through it —
/// so a run never leaves debris for the next one either.
struct SmokeDir(PathBuf);

impl SmokeDir {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "orbital_android_native_smoke-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for SmokeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gradle_build_smoke() {
    if std::env::var("ORBITAL_NATIVE_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipping gradle_build_smoke: set ORBITAL_NATIVE_SMOKE=1 to run");
        return;
    }

    let android_home = std::env::var("ANDROID_HOME")
        .or_else(|_| std::env::var("ANDROID_SDK_ROOT"))
        .ok();
    let Some(android_home) = android_home else {
        eprintln!("skipping gradle_build_smoke: neither ANDROID_HOME nor ANDROID_SDK_ROOT is set");
        return;
    };
    if !PathBuf::from(&android_home).exists() {
        eprintln!(
            "skipping gradle_build_smoke: ANDROID_HOME/ANDROID_SDK_ROOT ({}) does not exist",
            android_home
        );
        return;
    }

    let workspace_root = workspace_root();

    let native_lib_dir = workspace_root.join("target/jniLibs");
    let native_lib = native_lib_dir.join("arm64-v8a/liborbital_ffi.so");
    if !native_lib.exists() {
        eprintln!(
            "skipping gradle_build_smoke: {} not built — run `make android-lib` first (this test never builds it)",
            native_lib.display()
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
    // build. Prefer a release binary (matches `make release`/`make
    // android-lib` workflow); fall back to debug.
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
                "skipping gradle_build_smoke: orbital CLI not built (run `cargo build --release -p orbital-cli`)"
            );
            return;
        }
    };

    let smoke_dir = SmokeDir::new();
    let out_dir = smoke_dir.0.clone();

    let compile = Command::new(&binary)
        .args([
            "compile",
            fixture.to_str().unwrap(),
            "--shell",
            "android",
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .env("ORBITAL_NATIVE_LIB_DIR", &native_lib_dir)
        .env("ANDROID_HOME", &android_home)
        .output()
        .expect("`orbital compile` failed to launch");
    assert!(
        compile.status.success(),
        "orbital compile --shell android failed:\nSTDOUT:\n{}\nSTDERR:\n{}",
        tail(&String::from_utf8_lossy(&compile.stdout), 60),
        tail(&String::from_utf8_lossy(&compile.stderr), 60)
    );
    assert_binary_carries_this_crates_renderers(&binary, &out_dir);

    let gradlew = out_dir.join("gradlew");
    assert!(
        gradlew.exists(),
        "gradlew missing from generated project at {}",
        gradlew.display()
    );
    let copied_lib = out_dir.join("app/src/main/jniLibs/arm64-v8a/liborbital_ffi.so");
    assert!(
        copied_lib.exists(),
        "init_project did not copy the native lib into jniLibs at {}",
        copied_lib.display()
    );

    let build = Command::new("./gradlew")
        .arg("assembleDebug")
        .current_dir(&out_dir)
        .env("ANDROID_HOME", &android_home)
        .output()
        .expect("./gradlew failed to launch");
    assert!(
        build.status.success(),
        "./gradlew assembleDebug failed:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
        tail(&String::from_utf8_lossy(&build.stdout), 80),
        tail(&String::from_utf8_lossy(&build.stderr), 80)
    );
    eprintln!(
        "gradle_build_smoke: assembleDebug OK in {}",
        out_dir.display()
    );

    // Program C wave C4 render gate (docs/Almadar_Compiler_Gaps.md §157):
    // PatternSnapshotTests.kt, generated into app/src/test/kotlin/generated/
    // by AndroidBackend, snapshots every non-deferred pattern (Robolectric +
    // Roborazzi) against the fixtures `almadar-sync pattern-fixtures`
    // embeds. Baselines live checked in at
    // test-schemas/native-conformance/snapshots/android/<name>.png — copy
    // them into Roborazzi's captureRoboImage path (app/src/test/snapshots/)
    // before `verifyRoborazziDebug` so a normal (non-recording) run compares
    // against them. `ORBITAL_SNAPSHOT_RECORD=1` runs `recordRoborazziDebug`
    // instead and, on success, copies the freshly recorded images back out
    // to become the new checked-in baseline.
    let snapshot_baseline_dir =
        workspace_root.join("test-schemas/native-conformance/snapshots/android");
    let generated_snapshot_dir = out_dir.join("app/src/test/snapshots");
    std::fs::create_dir_all(&generated_snapshot_dir).expect("create generated snapshot dir");
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
        let baseline: Vec<PathBuf> = std::fs::read_dir(&snapshot_baseline_dir)
            .expect("read snapshot baseline dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
            .collect();
        blank_png::assert_no_blank_renders(&baseline, None, pattern_of_snapshot);
    }
    let roborazzi_task = if recording {
        "recordRoborazziDebug"
    } else {
        "verifyRoborazziDebug"
    };

    // The Rust-drawn canvases render in the JVM through the host build of `orbital_ffi`
    // (`cargo build -p orbital-ffi --release`); this test never builds it.
    let host_lib_dir = workspace_root.join("target/release");
    assert!(
        host_lib_dir.join("liborbital_ffi.dylib").exists() || host_lib_dir.join("liborbital_ffi.so").exists(),
        "host liborbital_ffi not built in {} — run `cargo build -p orbital-ffi --release` (the snapshot suite draws canvases through it)",
        host_lib_dir.display()
    );

    let roborazzi = Command::new("./gradlew")
        .arg(roborazzi_task)
        .current_dir(&out_dir)
        .env("ANDROID_HOME", &android_home)
        .env("ORBITAL_HOST_LIB_DIR", &host_lib_dir)
        .output()
        .expect("./gradlew roborazzi task failed to launch");

    // Gradle's `testDebugUnitTest` fails the WHOLE task the moment any one
    // `@Test` throws — including a pre-C5 placeholder body genuinely
    // crashing on a generic (non-curated) fixture's shape (§157 wave C5
    // rewrites every body; today's job is only to prove the harness itself
    // works). A recording run therefore does not gate on Gradle's exit
    // code, the same rationale as `xcode_build_smoke.rs`'s record-mode
    // handling — it gates on "did captureRoboImage produce PNGs at all".
    // A non-recording (verify) run keeps the exit-code gate: `verifyRoborazziDebug`
    // failing IS the signal a checked-in baseline regressed.
    if !recording {
        assert!(
            roborazzi.status.success(),
            "./gradlew {roborazzi_task} failed:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
            tail(&String::from_utf8_lossy(&roborazzi.stdout), 120),
            tail(&String::from_utf8_lossy(&roborazzi.stderr), 120)
        );
        eprintln!("gradle_build_smoke: {roborazzi_task} OK");
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
        "recording run produced no PNGs at {} — ./gradlew {roborazzi_task} tail:\nSTDOUT (tail):\n{}\nSTDERR (tail):\n{}",
        generated_snapshot_dir.display(),
        tail(&String::from_utf8_lossy(&roborazzi.stdout), 120),
        tail(&String::from_utf8_lossy(&roborazzi.stderr), 120)
    );

    let fixture_dir = workspace_root.join("test-schemas/native-conformance/pattern-fixtures");
    let mut missing: Vec<String> = std::fs::read_dir(&fixture_dir)
        .expect("read pattern fixture dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .filter(|name| {
            !recorded
                .iter()
                .any(|r| r.file_stem().and_then(|s| s.to_str()) == Some(name.as_str()))
        })
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "recording run produced no PNG for {} fixture(s): {}",
        missing.len(),
        missing.join(", ")
    );

    blank_png::assert_no_blank_renders(&recorded, None, pattern_of_snapshot);

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
        "gradle_build_smoke: recorded {} baseline(s) to {}",
        recorded.len(),
        snapshot_baseline_dir.display()
    );
}

/// The generated project's `CorePatternRegistry` is byte-for-byte the installed pack's: a binary built
/// before the renderers changed would record or compare stale snapshots.
fn assert_binary_carries_this_crates_renderers(binary: &Path, out_dir: &Path) {
    let pack = orbital_shell_android::require_renderer_pack().expect("installed renderer pack");
    let source = Path::new("CorePatternRegistry.kt");
    let expected = pack
        .source("CorePatternRegistry.kt")
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
