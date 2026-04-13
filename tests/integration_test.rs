use std::process::Command;

const INPUT: &str = "data/domains/cogsci/dials.json";
const RULES: &str = "../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites";

fn fixtures_present() -> bool {
    std::path::Path::new(INPUT).exists() && std::path::Path::new(RULES).exists()
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_egg-stitch"))
}

fn run_smc(extra_args: &[&str]) -> String {
    let mut cmd = bin();
    cmd.args(["--input", INPUT, "--rules", RULES]);
    cmd.args(extra_args);
    let output = cmd.output().expect("failed to run binary");
    assert!(output.status.success(), "binary exited with error: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("non-utf8 stdout")
}

fn extract_pattern(stdout: &str) -> &str {
    stdout.lines()
        .find(|l| l.contains("pattern:"))
        .and_then(|l| l.split("pattern:").nth(1))
        .map(|s| s.trim())
        .expect("no pattern: line in output")
}

const DIALS_FULL_FOLLOW: &str = "(T (T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0)) (M 1 0 (* ?#0 (* 0.5 (cos (/ pi 4)))) (* ?#0 (* 0.5 (sin (/ pi 4))))))";

/// Full follow baseline. Asserts the exact target pattern is found.
#[test]
#[ignore = "slow: 1000 steps * 1000 particles; run with --release --ignored"]
fn follow_dials_full_baseline() {
    if !fixtures_present() { return; }
    let stdout = run_smc(&[
        "--num-steps", "1000", "--num-particles", "1000", "--temperature", "1000",
        "--follow", DIALS_FULL_FOLLOW, "--max-arity", "2",
    ]);
    let pattern = extract_pattern(&stdout);
    assert_eq!(pattern, DIALS_FULL_FOLLOW, "expected exact target pattern");
}

/// Shallow follow with no variables — fast.
#[test]
fn follow_shallow_no_placeholders() {
    if !fixtures_present() { return; }
    let stdout = run_smc(&[
        "--num-steps", "30", "--num-particles", "200",
        "--follow", "(T l (M 1 0 -0.5 0))", "--max-arity", "2",
    ]);
    assert!(extract_pattern(&stdout).contains("T"), "pattern should contain T");
}

/// Follow with a `?#0` variable — verifies the search doesn't crash.
#[test]
#[ignore = "slow: 1000 steps * 1000 particles; run with --release --ignored"]
fn follow_single_placeholder() {
    if !fixtures_present() { return; }
    let stdout = run_smc(&[
        "--num-steps", "1000", "--num-particles", "1000", "--temperature", "1000",
        "--follow", "(T (T l (M 1 0 -0.5 0)) (M ?#0 (/ pi 4) 0 0))", "--max-arity", "2",
    ]);
    assert!(stdout.contains("pattern:"), "should produce a pattern");
}

/// No follow — sanity check.
#[test]
fn no_follow_still_produces_best() {
    if !fixtures_present() { return; }
    let stdout = run_smc(&["--num-steps", "20", "--num-particles", "100", "--max-arity", "2"]);
    assert!(stdout.contains("pattern:"), "should produce a pattern");
}
