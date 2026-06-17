//! Memory-blowup regression for factored substitution sets.
//!
//! The fixture `data/test/nuts-bolts-subst-blowup.rewrites` is the nuts-bolts
//! DSR set plus the `trans_combine` / `scale_combine` rules that collapse nested
//! `T` wrappers. Saturating with those rules makes many e-classes match the
//! abstraction pattern with *independent* variable slots, so the substitution
//! set at a match is the cartesian product of several factors.
//!
//! Before factoring, a match stored that product flattened (`∏|factor|` substs);
//! the factored representation stores the factors themselves (`Σ|factor|` rows).
//! This test pins that win: best-first is run under a hard address-space ceiling
//! that the factored build clears comfortably and the flat one cannot.
//!
//! The run is bounded by `--max-forced-expansion 1` so it drains the small
//! forced≈0 subspace — which is exactly where the many-independent-slot blowup
//! patterns live (they are general, hence low forced-expansion). Without that
//! bound the default `forced-then-cost` ordering explores whole forced-levels
//! broadly and holds a huge frontier of those big-factored-match-set patterns on
//! the heap at once (~1.5 GB), which would swamp the factored-vs-flat signal.
//! With the bound, the factored build peaks around ~375 MB here and the flat one
//! would need multiple GB.
//!
//! Determinism: best-first enumerates patterns deterministically (no rng), so
//! both the search and its peak memory are reproducible run to run.
//!
//! Linux-only: the ceiling is enforced with `ulimit -v` (RLIMIT_AS), whose
//! semantics and the glibc virtual-memory footprint the thresholds were
//! calibrated against are Linux-specific. Elsewhere the test is a no-op.

#![cfg(target_os = "linux")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");
const INPUT: &str = "data/domains/cogsci/nuts-bolts.json";
const RULES: &str = "data/test/nuts-bolts-subst-blowup.rewrites";

/// Address-space ceiling (KB) the run executes under. With the
/// `--max-forced-expansion 1` bound the factored build peaks around ~375 MB and
/// the flat build needs multiple GB, so 700 MB sits in that gap with ample
/// headroom over the factored peak to absorb allocator/CI variance.
const ADDR_SPACE_CAP_KB: u32 = 700_000;

/// Runs best-first on the blowup fixture under the `ADDR_SPACE_CAP_KB` ceiling.
/// The factored representation finishes within it; the flat one aborts when an
/// allocation pushes the address space over the limit.
#[test]
fn factored_substs_stay_under_memory_cap() {
    if !std::path::Path::new(INPUT).exists() || !std::path::Path::new(RULES).exists() {
        return;
    }

    // `ulimit -v` caps the shell and the exec'd binary; `exec "$0" "$@"` forwards
    // the program path and args verbatim, so paths with spaces stay intact.
    let script = format!("ulimit -v {ADDR_SPACE_CAP_KB}; exec \"$0\" \"$@\"");
    let status = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg(BIN)
        // Pin `--iter-limit`/`--node-limit` to the saturation bounds this test
        // was calibrated against (the former hardcoded defaults). The production
        // defaults are far larger, which would saturate the fixture into a much
        // bigger egraph and trip the cap on the build alone, independent of how
        // substitution sets are stored.
        .args(["--search", "best-first", "--input", INPUT, "--rules", RULES, "--num-abstractions", "1", "--max-arity", "2", "--num-steps", "300", "--max-forced-expansion", "1", "--iter-limit", "10", "--node-limit", "10000"])
        .status()
        .expect("spawn egg-stitch under ulimit");

    assert!(status.success(), "best-first on {RULES} exceeded the {ADDR_SPACE_CAP_KB} KB address-space cap (status {status:?}); the substitution set is not being stored factored");
}
