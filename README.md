<div align="center">

# orbital-integration-tests

**Cross-crate tests: the ones that need several Orbital crates at once.**

[![CI](https://github.com/almadar-io/orbital-integration-tests/actions/workflows/ci.yml/badge.svg)](https://github.com/almadar-io/orbital-integration-tests/actions/workflows/ci.yml) [![License: BUSL-1.1](https://img.shields.io/badge/license-BUSL--1.1-blue)](LICENSE)

</div>

<p align="center"><sub>Part of <a href="https://github.com/almadar-io/orb"><b>Orb</b></a>: a programming language of composable state machines. <a href="https://orb.almadar.io">orb.almadar.io</a></sub></p>

This repo holds the tests that need several crates at once, for example the compiler plus the shells. It is not a node in the architecture diagram, so nothing is highlighted. It is not published (`publish = false`). The library `orbital_test_support` is the shared home for fixture helpers (discovering and faking installed behavior packages) that `orbital-compiler`, `orbital-lolo` and `orbital-cli` use as a dev-dependency.

## Where this fits

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/almadar-io/orb/main/static/img/readme/architecture/orbital-integration-tests-dark.svg">
  <img src="https://raw.githubusercontent.com/almadar-io/orb/main/static/img/readme/architecture/orbital-integration-tests-light.svg" width="100%" alt="Where orbital-integration-tests sits in the Almadar architecture">
</picture>

| Upstream (this repo depends on) | Downstream (depends on this repo) |
|---|---|
| [orbital-core](https://github.com/almadar-io/orbital-core) and [orbital-lolo](https://github.com/almadar-io/orbital-lolo): the library's dependencies | [orbital-lolo](https://github.com/almadar-io/orbital-lolo), [orbital-compiler](https://github.com/almadar-io/orbital-compiler), [orbital-cli](https://github.com/almadar-io/orbital-cli): dev-dependency for the shared test helpers |
| Dev-dependencies for the tests: [orbital-compiler](https://github.com/almadar-io/orbital-compiler), [orbital-ffi](https://github.com/almadar-io/orbital-ffi), [orbital-canvas](https://github.com/almadar-io/orbital-canvas), [orbital-shell-typescript](https://github.com/almadar-io/orbital-shell-typescript), [orbital-shell-rust](https://github.com/almadar-io/orbital-shell-rust), [orbital-shell-ios](https://github.com/almadar-io/orbital-shell-ios), [orbital-shell-android](https://github.com/almadar-io/orbital-shell-android) | |

## Quick start

```bash
npm install --no-save --no-package-lock @almadar/core @almadar/std @almadar/lolo-ar @almadar/lolo-sl
cargo test
```

## What's inside

| Path | Contents |
|---|---|
| `src/lib.rs` | `orbital_test_support`: resolving the installed behavior packages and building minimal fake ones for tests |
| `tests/shell_program_parity.rs` | A shell program writes the same app as the in-process shell |
| `tests/ir_wire_parity.rs`, `tests/lolo_parity_harness.rs`, `tests/ffi_resolved_fixtures.rs` | IR, `.lolo` and FFI parity against shared fixtures |
| `tests/workspace_grepwall.rs` | Single-owner gates: a canonical helper must have exactly one home |
| `tests/native_*.rs`, `tests/gradle_build_smoke.rs`, `tests/xcode_build_smoke.rs`, `tests/shell_template_toolchain.rs` | Native shell and template checks |
| `fixtures/`, `tests/fixtures/` | Programs and expected outputs |

Some tests read `test-schemas/` and other paths of the monorepo checkout, and the native build smokes need Gradle or Xcode, so a full run is a monorepo job.

## Development

The crate reads the Almadar language data (patterns, operators, themes, vocabularies) from installed npm packages, so install those first (this is the CI recipe; the toolchain is pinned to Rust 1.93.0 in `rust-toolchain.toml`):

```bash
npm install --no-save --no-package-lock @almadar/core @almadar/std @almadar/lolo-ar @almadar/lolo-sl
cargo test
```

Its `orbital-*` dependencies are git dependencies on the `main` branch of their own repos.

Open gaps are tracked in [Almadar_Gaps.md](Almadar_Gaps.md). Work on `main`; never create a feature branch. Inside the monorepo this repo is the submodule `orbital-rust/crates/orbital-integration-tests`; work from `orbital-rust` with `cargo test -p orbital-integration-tests`.

## Docs

- [Orb documentation](https://orb.almadar.io/docs): the language, the `orb` CLI and how the pieces fit

## License

[Business Source License 1.1](LICENSE): free for non-commercial use and internal evaluation; commercial production use needs a licence from Almadar FZE. Converts to Apache-2.0 on 2030-02-01.
