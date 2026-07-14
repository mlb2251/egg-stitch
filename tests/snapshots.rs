//! Data-driven snapshot (bless/check) suite. Every case lives in
//! `tests/snapshots.toml`; this harness turns each into one libtest-mimic
//! trial, so there is no handwritten Rust test per fixture. A `coverage` trial
//! enforces that the manifest and the on-disk fixture tree are in exact
//! correspondence — a fixture with no case (or a case with no fixture) fails.
//!
//! Bless/check, run, and filter exactly like an ordinary test binary:
//!
//! ```text
//! cargo test --release --test snapshots                 # check all
//! cargo test --release --test snapshots -- cogsci       # check a subset
//! BLESS=1 cargo test --release --test snapshots          # re-bless all
//! ```

use libtest_mimic::{Arguments, Failed, Trial};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod common;

const FIXTURE_ROOT: &str = "data/expected_outputs";

fn main() {
    let args = Arguments::from_args();
    let manifest = common::load_manifest();

    let mut trials: Vec<Trial> = manifest
        .cases
        .into_iter()
        .map(|case| {
            // `foo/bar` → `foo::bar` so trials group like modules in output.
            let name = case.name.replace('/', "::");
            Trial::test(name, move || case.run().map_err(Failed::from))
        })
        .collect();

    // Coverage is a check-mode guardrail; under BLESS the fixtures are being
    // (re)written concurrently, so an on-disk snapshot would race the writers.
    if std::env::var("BLESS").is_err() {
        trials.push(Trial::test("coverage::manifest_matches_fixtures", coverage));
    }

    libtest_mimic::run(&args, trials).exit();
}

/// Fails if the set of fixtures the manifest declares differs from the set of
/// `*.out.json` files on disk. Reports the offending paths on either side.
fn coverage() -> Result<(), Failed> {
    let manifest = common::load_manifest();
    let declared: BTreeSet<String> = manifest.cases.iter().map(|c| c.fixture_path()).collect();

    let mut on_disk = BTreeSet::new();
    collect_fixtures(Path::new(FIXTURE_ROOT), &mut on_disk);

    let orphan_fixtures: Vec<&String> = on_disk.difference(&declared).collect();
    let missing_fixtures: Vec<&String> = declared.difference(&on_disk).collect();
    if orphan_fixtures.is_empty() && missing_fixtures.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    if !orphan_fixtures.is_empty() {
        msg.push_str(&format!("{} fixture(s) with no case in tests/snapshots.toml:\n", orphan_fixtures.len()));
        for p in orphan_fixtures {
            msg.push_str(&format!("  {p}\n"));
        }
    }
    if !missing_fixtures.is_empty() {
        msg.push_str(&format!("{} case(s) whose fixture is absent (run with BLESS=1 to create):\n", missing_fixtures.len()));
        for p in missing_fixtures {
            msg.push_str(&format!("  {p}\n"));
        }
    }
    Err(Failed::from(msg))
}

/// Recursively collects every `*.out.json` under `dir` into `out`.
fn collect_fixtures(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path: PathBuf = entry.path();
        if path.is_dir() {
            collect_fixtures(&path, out);
        } else if path.to_str().is_some_and(|p| p.ends_with(".out.json")) {
            out.insert(path.to_string_lossy().into_owned());
        }
    }
}
