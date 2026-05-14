//! Regression tests where we run egg-stitch end-to-end on a small fixture and
//! verify the rewritten programs β-reduce back to the originals via
//! `scripts/check_equiv.py`. Each test pins a small set of SMC seeds; if any
//! seed's output is rejected by the checker the test fails.
//!
//! These cover unsoundness bugs that pure structural fixture comparison
//! (`stitch_compat_test`) can't see — the rewriting can produce a result
//! that's structurally well-formed but semantically diverges from the input.

use std::{fs, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

fn checker_path() -> String {
    format!("{}/scripts/check_equiv.py", env!("CARGO_MANIFEST_DIR"))
}

/// Runs egg-stitch on `input` with the given seed, then runs `check_equiv.py`
/// on the output. Returns Ok(()) if both succeed, Err(message) otherwise.
fn run_and_check(input: &str, seed: u64) -> Result<(), String> {
    let out = std::env::temp_dir().join(format!("egg-stitch-check-equiv-{}-{}.json", std::process::id(), seed));
    let out_str = out.to_str().expect("utf-8 temp path");

    let status = Command::new(BIN)
        .args([
            "-i",
            input,
            "--output",
            out_str,
            "--search",
            "smc",
            "--language",
            "lambda-calc",
            "--num-abstractions",
            "1",
            "--num-particles",
            "1000",
            "--num-steps",
            "1000",
            "--temperature",
            "1000",
            "--seed",
        ])
        .arg(seed.to_string())
        .output()
        .map_err(|e| format!("spawn egg-stitch: {e}"))?;
    if !status.status.success() {
        return Err(format!("egg-stitch failed for seed={seed}: {}", String::from_utf8_lossy(&status.stderr)));
    }

    let check = Command::new("python3").arg(checker_path()).arg(out_str).output().map_err(|e| format!("spawn check_equiv: {e}"))?;
    let _ = fs::remove_file(&out);
    if !check.status.success() {
        return Err(format!("seed={seed}: check_equiv rejected output:\n{}{}", String::from_utf8_lossy(&check.stdout), String::from_utf8_lossy(&check.stderr)));
    }
    Ok(())
}

fn sweep(input: &str, seeds: &[u64]) {
    let mut failures: Vec<String> = Vec::new();
    for &seed in seeds {
        if let Err(msg) = run_and_check(input, seed) {
            failures.push(msg);
        }
    }
    if !failures.is_empty() {
        panic!("{} seed(s) failed:\n{}", failures.len(), failures.join("\n---\n"));
    }
}

/// Regression for the per-slot-memo leak in `wrap_subst_args`.
///
/// `permuted_shift_egraph` is parameterised by the slot's
/// `(d_k, h, rank_map)`, but its memo originally keyed only on
/// `(canonical, initial_depth)`. When a sub-eclass shared by two slots was
/// first transformed for the slot with `d_k > 0` (mapping `$i → $rank`), a
/// later visit from the slot with `d_k = 0` (which should be identity)
/// returned the cached transformed result instead — silently dropping `$1`s.
///
/// The physics fixture exposes this: `(fold ?#0 0. (lam (lam (+. $0 ?#1))))`
/// has `?#0` at d_k=0 and `?#1` at d_k=2. The captured arg for `?#0`'s
/// `(zip $0 $1 …)` shares the `$1` eclass with `?#1`'s capture, so the
/// `$1`-as-`?#1`-capture memo entry leaked into the `?#0`-as-`zip`-child
/// recursion.
#[test]
fn physics_fold_zip_capture() {
    let input = "data/domains/ho-bugs/zip_fold_capture.json";
    sweep(input, &[0, 1, 2, 3, 4, 5, 6, 7]);
}
