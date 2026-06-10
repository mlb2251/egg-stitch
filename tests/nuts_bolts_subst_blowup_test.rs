//! Memory-blowup regression for factored substitution sets.
//!
//! The fixture `data/test/nuts-bolts-subst-blowup.rewrites` is the nuts-bolts
//! DSR set plus the `trans_combine` / `scale_combine` rules that collapse nested
//! `T` wrappers. Saturating with those rules makes many e-classes match the
//! abstraction pattern with *independent* variable slots, so the substitution
//! set at a match is the cartesian product of several factors.
//!
//! Before factoring, a match stored that product flattened (`∏|factor|` substs),
//! which blows the process up to >200 MB on this corpus. The factored
//! representation stores the factors themselves (`Σ|factor|` rows), keeping the
//! same search under ~55 MB. This test pins that win: best-first is run under a
//! hard address-space ceiling that the factored build clears comfortably and the
//! flat build cannot.
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

/// Address-space ceiling (KB) the run executes under. The factored build peaks
/// around 53 MB here and the flat build needs >200 MB, so 130 MB sits centrally
/// in that gap — ~75 MB of headroom on each side absorbs allocator/CI variance.
const ADDR_SPACE_CAP_KB: u32 = 130_000;

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
        //
        // `--no-opt-canonical-seen` pins the search trajectory too: the canonical
        // seen-set's dedup reorders the best-first frontier, and on this corpus —
        // engineered to contain patterns with huge cartesian-product subst sets —
        // that reordering pops a heavy node within the step budget and balloons
        // memory (~1.5 GB), masking the factored-vs-flat property this test
        // actually measures. It's a no-op on normal corpora (see canonical.rs).
        .args([
            "--search",
            "best-first",
            "--input",
            INPUT,
            "--rules",
            RULES,
            "--num-abstractions",
            "1",
            "--max-arity",
            "2",
            "--num-steps",
            "300",
            "--iter-limit",
            "10",
            "--node-limit",
            "10000",
            "--no-opt-canonical-seen",
        ])
        .status()
        .expect("spawn egg-stitch under ulimit");

    assert!(status.success(), "best-first on {RULES} exceeded the {ADDR_SPACE_CAP_KB} KB address-space cap (status {status:?}); the substitution set is not being stored factored");
}
