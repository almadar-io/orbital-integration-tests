//! Shared test fixtures for discovering/faking installed behavior packages.
//!
//! Phase 2 (`docs/Almadar_Behavior_Packages.md` §10): `@almadar/std` and
//! `@almadar-io/behaviors` are installed npm packages, not baked into the
//! compiler. Tests across `orbital-compiler`, `orbital-lolo`, `orbital-cli`
//! and `orbital-verify` need to (a) resolve the real installed packages from
//! `CARGO_MANIFEST_DIR` in the monorepo, never from a hand-rolled `../packages`
//! walk-up, and (b) fabricate a minimal installed package on disk for unit
//! tests that must not depend on the monorepo tree at all. This crate is the
//! one shared home for both so no crate keeps its own copy.

use orbital_core::behavior_packages::{discover, user_store_dir, BehaviorPackage, RegistrySource};
use orbital_core::language_data::{
    install_language_data, language_data, load_language_data, LanguageData,
};
use orbital_core::schema::OrbitalSchema;
use std::fs;
use std::path::{Path, PathBuf};

/// Maps a behavior-package `orb.prefix` to the npm package name shown in
/// install hints; an unrecognized prefix is echoed back as-is.
fn install_hint(prefix: &str) -> String {
    match prefix {
        "almadar-std" => "@almadar/std".to_string(),
        "almadar-behaviors" => "@almadar-io/behaviors".to_string(),
        other => other.to_string(),
    }
}

/// Resolves the installed behavior package whose `orb.prefix` is `prefix`,
/// walking node-style from `start` (project `node_modules` first, the user
/// store as the last rung). Panics with an actionable message naming the
/// npm install command when it isn't discoverable — a standalone checkout,
/// or one that hasn't run `orb update`.
pub fn installed_package(prefix: &str, start: &Path) -> BehaviorPackage {
    let user_store = user_store_dir().unwrap_or_default();
    let packages = discover(start, &user_store).expect("behavior package discovery failed");
    packages
        .into_iter()
        .find(|p| p.prefix == prefix)
        .unwrap_or_else(|| {
            panic!(
                "behavior package with prefix `{prefix}` is not installed/discoverable from {} \
                 — run `orb add {}` (or `orb update --global`)",
                start.display(),
                install_hint(prefix),
            )
        })
}

/// Finds `<name>.orb` anywhere under the installed `prefix` package's
/// registry directory (recursive walk). Panics — listing every match —
/// when there isn't exactly one.
pub fn installed_orb(prefix: &str, start: &Path, name: &str) -> PathBuf {
    let pkg = installed_package(prefix, start);
    let target = format!("{name}.{}", pkg.registry.extension());
    let mut matches = Vec::new();
    walk_behavior_files(pkg.registry.root(), pkg.registry.extension(), &mut |path| {
        if path.file_name().and_then(|f| f.to_str()) == Some(target.as_str()) {
            matches.push(path.to_path_buf());
        }
    });
    match matches.len() {
        1 => matches.into_iter().next().expect("checked len == 1"),
        0 => panic!(
            "no `{target}` found under {} (package {}@{})",
            pkg.registry.root().display(),
            pkg.name,
            pkg.version
        ),
        n => panic!(
            "expected exactly one `{target}` under {}, found {n}: {matches:?}",
            pkg.registry.root().display()
        ),
    }
}

/// The `orbital_compiler::loader::SourceLowerer` the `orb` binary registers,
/// for tests whose loader reads a live (`.lolo`) behavior package.
pub fn lower_lolo_file(path: &Path) -> Result<OrbitalSchema, String> {
    orbital_lolo::parse_and_lower(path).map_err(|e| e.to_string())
}

/// The installed `prefix` package's `name` behavior as a schema: lowered from
/// `.lolo` for a live package, parsed from `.orb` for a materialized one.
pub fn installed_schema(prefix: &str, start: &Path, name: &str) -> OrbitalSchema {
    let path = installed_orb(prefix, start, name);
    match installed_package(prefix, start).registry {
        RegistrySource::Live(_) => {
            lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower {}: {e}", path.display()))
        }
        RegistrySource::Materialized(_) => {
            let json = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let mut schema: OrbitalSchema = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            orbital_core::stamp::ensure_stamped(&mut schema);
            schema
        }
    }
}

/// `installed_orb` pinned to `@almadar/std` (`orb.prefix == "almadar-std"`).
pub fn installed_std_orb(start: &Path, name: &str) -> PathBuf {
    installed_orb("almadar-std", start, name)
}

/// `installed_orb` pinned to `@almadar-io/behaviors` (`orb.prefix ==
/// "almadar-behaviors"`).
pub fn installed_io_orb(start: &Path, name: &str) -> PathBuf {
    installed_orb("almadar-behaviors", start, name)
}

fn walk_behavior_files(dir: &Path, extension: &str, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_behavior_files(&path, extension, visit);
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            visit(&path);
        }
    }
}

/// Writes a minimal installed-package layout under
/// `<root>/node_modules/<name>`: `package.json` with an `orb` block, one
/// `.orb` file per `(rel_path, json)` in `orbs` under `behaviors/registry/`,
/// an empty `behaviors/lolo` source tree (`orb.source`, where the package's
/// contracts are derived from; add `.lolo` files there).
/// Returns `<root>/node_modules/<name>`.
pub fn fake_package(
    root: &Path,
    name: &str,
    prefix: &str,
    version: &str,
    orbs: &[(&str, &str)],
) -> PathBuf {
    let pkg_dir = root.join("node_modules").join(name);
    let registry_dir = pkg_dir.join("behaviors/registry");
    fs::create_dir_all(&registry_dir).expect("mkdir registry");

    for (rel_path, json) in orbs {
        let file = registry_dir.join(rel_path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("mkdir orb parent dir");
        }
        fs::write(&file, json).expect("write orb file");
    }

    fs::create_dir_all(pkg_dir.join("behaviors/lolo")).expect("mkdir lolo source");

    let pkg_json = serde_json::json!({
        "name": name,
        "version": version,
        "orb": {
            "prefix": prefix,
            "registry": "behaviors/registry",
            "source": "behaviors/lolo",
            "embeddings": [],
            "compiler": ">=0.0.0",
        },
    });
    fs::write(
        pkg_dir.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .expect("write package.json");

    pkg_dir
}

/// Installs the language data (the pattern vocabulary) from the `@almadar/core`
/// found by walking node-style from `start` (project `node_modules` first, the
/// user store last) as the process-wide data every L1/L2 accessor reads, and
/// returns it. Idempotent: a second call returns what is already installed.
/// Panics with the install hint when core is not installed/compatible — in the
/// monorepo the root `node_modules/@almadar/core` workspace link answers; a
/// standalone checkout gets it from `orbital-rust/package.json`.
pub fn installed_core_language(start: &Path) -> &'static LanguageData {
    if let Ok(data) = language_data() {
        return data;
    }
    let data = load_language_data(start, &user_store_dir().unwrap_or_default())
        .unwrap_or_else(|e| panic!("{}: {e} (searched from {})", e.code(), start.display()));
    let _ = install_language_data(data);
    language_data().expect("language data was just installed")
}

/// Links the installed `@almadar/core` (see [`link_installed_core`]) and writes
/// a language-only `@almadar/std` into `<project>/node_modules`: the installed
/// std's operator table under `orbLanguage`, with no behavior `orb` block, so a
/// spawned `orb` passes its language preflight while behavior discovery sees
/// only the project's own fixture packages.
#[cfg(unix)]
pub fn link_installed_language(project: &Path, start: &Path) {
    link_installed_core(project, start);
    let ops = orbital_core::language_data::load_operator_data(
        start,
        &user_store_dir().unwrap_or_default(),
    )
    .unwrap_or_else(|e| panic!("{}: {e} (searched from {})", e.code(), start.display()));
    let std_dir = project.join("node_modules/@almadar/std");
    fs::create_dir_all(&std_dir).expect("create node_modules/@almadar/std");
    fs::write(
        std_dir.join("package.json"),
        serde_json::json!({
            "name": "@almadar/std",
            "version": ops.std_version,
            "orbLanguage": { "compiler": ">=0.0.0", "operators": "canonical-operators.json" }
        })
        .to_string(),
    )
    .expect("write language-only @almadar/std package.json");
    fs::write(std_dir.join("canonical-operators.json"), &ops.operators)
        .expect("write canonical-operators.json");
}

/// Links every topic directory of the installed `@almadar/std`'s `.lolo` tree (found by walking from `start`) into
/// `<std_stub>/behaviors/lolo/<topic>`, beside any topic the test already wrote there, so `orb <command>` in a test
/// project finds std's command programs (`validate`, `lower`, ...) and everything they compose.
#[cfg(unix)]
pub fn link_installed_std_programs(std_stub: &Path, start: &Path) {
    let installed = installed_package("almadar-std", start);
    let source = installed
        .lolo_source
        .unwrap_or_else(|| panic!("the installed @almadar/std at {} carries no .lolo tree", installed.root.display()));
    let lolo = std_stub.join("behaviors/lolo");
    fs::create_dir_all(&lolo).expect("create the stub's .lolo tree");
    for entry in fs::read_dir(&source).expect("read the installed std's .lolo tree").flatten() {
        let link = lolo.join(entry.file_name());
        assert!(!link.exists(), "the test's own topic {} collides with an installed std topic", link.display());
        std::os::unix::fs::symlink(entry.path(), &link).expect("link an installed std topic");
    }
}

/// Symlinks the installed `@almadar/core` (found by walking from `start`) into
/// `<project>/node_modules/@almadar/core`, so a spawned `orb` running with
/// `project` as its cwd resolves the language data.
#[cfg(unix)]
pub fn link_installed_core(project: &Path, start: &Path) {
    let data = installed_core_language(start);
    let link = project.join("node_modules/@almadar/core");
    fs::create_dir_all(link.parent().expect("link parent")).expect("create node_modules/@almadar");
    std::os::unix::fs::symlink(&data.root, &link).expect("symlink installed @almadar/core");
}
