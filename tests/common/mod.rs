//! Shared infrastructure for the data-driven snapshot (bless/check) suite.
//!
//! Every bless/check case is the same pipeline: spawn the `egg-stitch` binary
//! with some args, write its `--output` JSON to a temp file, read it back,
//! strip the non-deterministic / bookkeeping fields, and bless-or-check the
//! result against a frozen fixture. The cases themselves live as data in
//! `tests/snapshots.toml`; `tests/snapshots.rs` turns each into a libtest-mimic
//! trial. The handful of tests that assert a *feature invariant* on top of the
//! snapshot (`tests/snapshot_asserts.rs`) reuse [`run`] and [`read_fixture`].
//!
//! To regenerate all fixtures after a legitimate behavior change:
//!
//! ```text
//! BLESS=1 cargo test --release --test snapshots
//! ```
#![allow(dead_code)]

use serde::Deserialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The cargo-built binary under test.
pub const BIN: &str = env!("CARGO_BIN_EXE_egg-stitch");

/// Top-level output fields that are non-deterministic (`timestamp`,
/// `elapsed_secs`, `iteration_times`) or just echo the invocation
/// (`input_file`, `rules_file`, `search`); stripped before comparison.
const STRIP_TOP: &[&str] = &["timestamp", "elapsed_secs", "iteration_times", "input_file", "rules_file", "search"];

/// Per-library-entry bookkeeping fields that vary with the search trace rather
/// than the abstraction found; stripped before comparison. `best_history` is
/// best-first-only (absent under SMC), so stripping it everywhere is a safe
/// no-op for SMC fixtures.
const STRIP_LIB: &[&str] = &["num_steps_run", "num_expansions", "best_iteration", "best_history"];

// ─── JSON key sorting ────────────────────────────────────────────────────────

/// Recursively sort every JSON object's keys alphabetically, in place, so
/// blessed fixtures are order-stable regardless of `serde_json`'s
/// `preserve_order` feature.
pub fn sort_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, child) in entries.iter_mut() {
                sort_keys(child);
            }
            *map = entries.into_iter().collect();
        }
        Value::Array(items) => items.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

/// Returns a clone of `value` with all object keys recursively sorted.
pub fn sorted(value: &Value) -> Value {
    let mut v = value.clone();
    sort_keys(&mut v);
    v
}

// ─── field stripping ─────────────────────────────────────────────────────────

/// Removes the non-deterministic top-level and per-library bookkeeping fields.
fn strip_nondeterministic(v: &mut Value) {
    if let Some(obj) = v.as_object_mut() {
        for k in STRIP_TOP {
            obj.remove(*k);
        }
    }
    for k in STRIP_LIB {
        strip_library_field(v, k);
    }
}

/// Strips a named field from every entry in `library` (in place).
pub fn strip_library_field(v: &mut Value, key: &str) {
    let Some(library) = v.get_mut("library").and_then(|l| l.as_array_mut()) else {
        return;
    };
    for entry in library {
        if let Some(obj) = entry.as_object_mut() {
            obj.remove(key);
        }
    }
}

/// Strips the `pattern` and `lambda` fields from every library entry. Used when
/// the chosen e-class representative is non-deterministic (e.g. once
/// commutativity rewrites unify several equivalent pattern strings); `lambda` is
/// derived from `pattern`, so they vary together.
fn strip_library_patterns(v: &mut Value) {
    strip_library_field(v, "pattern");
    strip_library_field(v, "lambda");
}

// ─── running the binary ──────────────────────────────────────────────────────

/// Unique temp `--output` path per call: several cases run the same (input,
/// search) with only differing flags, so a shared path would race parallel
/// trials. pid + a process-global counter guarantees uniqueness; the stem and
/// search are kept only for human readability.
fn temp_output_path(input: &str, search: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stem = Path::new(input).file_stem().and_then(|s| s.to_str()).unwrap_or("input");
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("egg-stitch-snap-{}-{}-{}-{}.json", std::process::id(), stem, search, n))
}

/// Spawns the binary with `--search <search> --input <input> --output <tmp>`
/// followed by `fixed` then `extra`, reads the output JSON back, and strips the
/// non-deterministic fields. Flag *spelling* (`-i`/`-r` vs `--input`/`--rules`)
/// doesn't affect the output, so the harness always uses the long forms.
fn run_raw(search: &str, input: &str, fixed: &[&str], extra: &[&str]) -> Value {
    let out = temp_output_path(input, search);
    let out_str = out.to_str().expect("utf-8 temp path");
    let mut cmd = Command::new(BIN);
    cmd.args(["--search", search, "--input", input, "--output", out_str]);
    cmd.args(fixed);
    cmd.args(extra);
    // The binary streams search progress to stdout; it inherits the harness's
    // fd (libtest-mimic can't capture a subprocess), so silence it to keep the
    // test log clean. stderr is left inherited so real errors still surface.
    cmd.stdout(std::process::Stdio::null());
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {BIN}: {e}"));
    assert!(status.success(), "{search} run failed for {input} (args: {fixed:?} {extra:?})");

    let text = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()));
    let _ = std::fs::remove_file(&out);
    let mut v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", out.display()));
    strip_nondeterministic(&mut v);
    v
}

/// The stitch-compat run convention: `--check-slow --num-abstractions 1`, with
/// best-first at a 50 000-step budget and SMC at 1000 particles × 1000 steps.
/// Exposed for the feature-assertion tests, which run a config and assert on the
/// result rather than snapshotting it.
pub fn run(search: &str, input: &str, extra: &[&str]) -> Value {
    run_stitch_backend(search, input, true, "1", "50000", extra)
}

/// One stitch-convention backend run: `[--check-slow] --num-abstractions <n>`,
/// with best-first at `bf_steps` and SMC at 1000 particles × 1000 steps.
fn run_stitch_backend(search: &str, input: &str, check_slow: bool, num_abstractions: &str, bf_steps: &str, extra: &[&str]) -> Value {
    let mut fixed: Vec<&str> = Vec::new();
    if check_slow {
        fixed.push("--check-slow");
    }
    fixed.extend(["--num-abstractions", num_abstractions]);
    if search == "best-first" {
        fixed.extend(["--num-steps", bf_steps]);
    } else {
        fixed.extend(["--num-particles", "1000", "--num-steps", "1000", "--temperature", "100"]);
    }
    run_raw(search, input, &fixed, extra)
}

// ─── bless / check ───────────────────────────────────────────────────────────

/// Blesses (`BLESS=1`) or checks `value` against the frozen fixture at `path`.
/// Returns `Err(msg)` on a check mismatch so the caller (a trial) can report it;
/// panics only on I/O the test can't recover from.
pub fn bless_or_check(path: &str, value: &Value) -> Result<(), String> {
    let value = sorted(value);
    if std::env::var("BLESS").is_ok() {
        let mut text = serde_json::to_string_pretty(&value).expect("serialize expected");
        text.push('\n');
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir {}: {e}", parent.display()));
        }
        std::fs::write(path, text).unwrap_or_else(|e| panic!("write {path}: {e}"));
        Ok(())
    } else {
        let text = std::fs::read_to_string(path).map_err(|e| format!("missing fixture {path}: {e} (run with BLESS=1 to create)"))?;
        let mut expected: Value = serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))?;
        sort_keys(&mut expected);
        if value == expected { Ok(()) } else { Err(format!("fixture mismatch for {path} (run with BLESS=1 to update)")) }
    }
}

/// Reads and parses a blessed fixture (for the feature-assertion tests).
pub fn read_fixture(path: &str) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

// ─── manifest schema ─────────────────────────────────────────────────────────

/// The per-suite run recipe: which backend(s) and fixed args a case uses. The
/// deliberate search budgets live here (one place) so the manifest only carries
/// what varies per case.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Stitch-compat: `--check-slow --num-abstractions 1`; best-first, or
    /// best-first + SMC collapsed to one fixture when they agree.
    Stitch,
    /// Cogsci drawing domains: best-first, `--num-abstractions 3 --max-arity 2`.
    Cogsci,
    /// Dreamcoder list/physics: best-first over a *glob* of per-benchmark files,
    /// aggregated into one fixture keyed by file name; lambda-calc.
    Dreamcoder,
    /// EPFL circuits: a fixed 4-abstraction SMC rollout (seed 1).
    Epfl,
}

/// Which backends a [`Kind::Stitch`] case runs.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Backends {
    /// Best-first only (SMC converges unreliably for this corpus).
    #[default]
    Bf,
    /// Best-first + SMC, collapsed to one fixture when they agree.
    Both,
}

fn default_true() -> bool {
    true
}

/// One snapshot case. Deserialized from a `[[case]]` table in the manifest.
#[derive(Deserialize)]
pub struct Case {
    /// Unique, `/`-structured case name (also the trial name, `/`→`::`).
    pub name: String,
    /// Which suite recipe supplies the fixed args.
    pub kind: Kind,
    /// Single input corpus (all kinds except a [`Kind::Dreamcoder`] glob).
    #[serde(default)]
    pub input: Option<String>,
    /// Glob of input corpora for [`Kind::Dreamcoder`] aggregation.
    #[serde(default)]
    pub glob: Option<String>,
    /// Explicit fixture path under `data/expected_outputs/` (no `.out.json`).
    /// Defaults to the input path with `data/[domains/]` and `.json` stripped.
    #[serde(default)]
    pub fixture: Option<String>,
    /// Stitch: which backend(s) to run.
    #[serde(default)]
    pub backends: Backends,
    /// Stitch: `--num-abstractions` (default "1"; roll-over stacks need 2+).
    #[serde(default)]
    pub num_abstractions: Option<String>,
    /// Stitch: pass `--check-slow` (default true; off for paths where the
    /// fast/slow cross-check isn't validated, e.g. molecule roll-over).
    #[serde(default = "default_true")]
    pub check_slow: bool,
    /// Best-first `--num-steps` budget override (default per kind).
    #[serde(default)]
    pub steps: Option<String>,
    /// `--rules <path>` for the run (also feeds the equivalence oracle).
    #[serde(default)]
    pub rules: Option<String>,
    /// EPFL: pass `--only-use-dsrs-at-start`.
    #[serde(default)]
    pub at_start: bool,
    /// Whether to keep the abstraction `pattern`/`lambda` fields (stitch only).
    #[serde(default = "default_true")]
    pub check_pattern: bool,
    /// When false, this is an oracle-only entry: `check_all_outputs.py` still
    /// checks the fixture, but no snapshot trial runs it (e.g. a corpus too
    /// large to search in a unit test). Keeps coverage a bijection.
    #[serde(default = "default_true")]
    pub snapshot: bool,
    /// Extra CLI args appended verbatim.
    #[serde(default)]
    pub args: Vec<String>,
    /// Equivalence-oracle spec, consumed by `scripts/check_all_outputs.py`.
    /// Ignored here.
    #[serde(default)]
    pub oracle: Option<toml::Value>,
}

/// The whole manifest: a list of `[[case]]` tables.
#[derive(Deserialize)]
pub struct Manifest {
    #[serde(default, rename = "case")]
    pub cases: Vec<Case>,
}

/// Loads and parses `tests/snapshots.toml`.
pub fn load_manifest() -> Manifest {
    let text = std::fs::read_to_string("tests/snapshots.toml").expect("read tests/snapshots.toml");
    toml::from_str(&text).expect("parse tests/snapshots.toml")
}

impl Case {
    /// The fixture file this case blesses/checks.
    pub fn fixture_path(&self) -> String {
        if let Some(f) = &self.fixture {
            return format!("data/expected_outputs/{f}.out.json");
        }
        let input = self.input.as_deref().expect("case needs `input` or explicit `fixture`");
        let rel = input.strip_prefix("data/domains/").or_else(|| input.strip_prefix("data/")).expect("input under data/domains/ or data/");
        let stem = rel.strip_suffix(".json").unwrap_or(rel);
        format!("data/expected_outputs/{stem}.out.json")
    }

    /// Extra args as `&str`, with `--rules <path>` prepended when set.
    fn extra_args(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(r) = &self.rules {
            v.push("--rules".to_string());
            v.push(r.clone());
        }
        v.extend(self.args.iter().cloned());
        v
    }

    /// Runs the case and blesses/checks it against its fixture.
    pub fn run(&self) -> Result<(), String> {
        let extra_owned = self.extra_args();
        let extra: Vec<&str> = extra_owned.iter().map(String::as_str).collect();
        let value = match self.kind {
            Kind::Stitch => self.run_stitch(&extra),
            Kind::Cogsci => self.run_cogsci(&extra),
            Kind::Dreamcoder => self.run_dreamcoder(&extra),
            Kind::Epfl => self.run_epfl(&extra),
        };
        bless_or_check(&self.fixture_path(), &value)
    }

    fn input(&self) -> &str {
        self.input.as_deref().expect("case needs `input`")
    }

    /// Stitch recipe: best-first (and optionally SMC, collapsed) with
    /// `--check-slow --num-abstractions 1`.
    fn run_stitch(&self, extra: &[&str]) -> Value {
        let steps = self.steps.as_deref().unwrap_or("50000");
        let na = self.num_abstractions.as_deref().unwrap_or("1");
        let mut bf = run_stitch_backend("best-first", self.input(), self.check_slow, na, steps, extra);
        if !self.check_pattern {
            strip_library_patterns(&mut bf);
        }
        if self.backends == Backends::Bf {
            return bf;
        }
        let mut smc = run_stitch_backend("smc", self.input(), self.check_slow, na, steps, extra);
        if !self.check_pattern {
            strip_library_patterns(&mut smc);
        }
        // Collapse to a single entry when both backends agree; else record both
        // side-by-side. `heap_sizes_at_end` is best-first-only, so a bare
        // difference on it isn't a real divergence — compare with it removed and
        // keep the best-first value (which carries it) when they otherwise agree.
        let agree = {
            let mut bf_cmp = bf.clone();
            if let Some(obj) = bf_cmp.as_object_mut() {
                obj.remove("heap_sizes_at_end");
            }
            bf_cmp == smc
        };
        if agree { bf } else { serde_json::json!({"best-first": bf, "smc": smc}) }
    }

    /// Cogsci recipe: best-first, `--num-abstractions 3 --max-arity 2`.
    fn run_cogsci(&self, extra: &[&str]) -> Value {
        let steps = self.steps.as_deref().unwrap_or("50000");
        run_raw("best-first", self.input(), &["--num-abstractions", "3", "--max-arity", "2", "--num-steps", steps], extra)
    }

    /// Dreamcoder recipe: best-first lambda-calc over every `*.json` matched by
    /// `glob`, aggregated into one object keyed by file name.
    fn run_dreamcoder(&self, extra: &[&str]) -> Value {
        let steps = self.steps.as_deref().unwrap_or("50000");
        let mut map = Map::new();
        for path in glob_json(self.glob.as_deref().expect("dreamcoder case needs `glob`")) {
            let name = path.file_name().and_then(|n| n.to_str()).expect("utf-8 file name").to_string();
            let input = path.to_str().expect("utf-8 input path");
            let v = run_raw("best-first", input, &["--language", "lambda-calc", "--num-abstractions", "3", "--max-arity", "2", "--num-steps", steps], extra);
            map.insert(name, v);
        }
        Value::Object(map)
    }

    /// EPFL recipe: a fixed 4-abstraction SMC rollout at seed 1.
    fn run_epfl(&self, extra: &[&str]) -> Value {
        let mut fixed = vec![
            "--language",
            "op-children-db",
            "--max-arity",
            "4",
            "--num-abstractions",
            "4",
            "--num-particles",
            "500",
            "--num-steps",
            "100",
            "--temperature",
            "100",
            "--seed",
            "1",
            "--iter-limit",
            "30",
        ];
        if self.at_start {
            fixed.push("--only-use-dsrs-at-start");
        }
        run_raw("smc", self.input(), &fixed, extra)
    }
}

/// All `*.json` files matching a `dir/*.json` glob, sorted for determinism.
fn glob_json(glob: &str) -> Vec<PathBuf> {
    let dir = glob.strip_suffix("/*.json").unwrap_or_else(|| panic!("glob must end with /*.json: {glob}"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir}: {e}")).filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    assert!(!files.is_empty(), "no *.json inputs match {glob}");
    files
}
