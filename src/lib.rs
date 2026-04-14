pub mod best_first;
pub mod cost;
pub mod debug_log;
pub mod follow;
pub mod io;
pub mod lang;
pub mod logging;
pub mod matching;
pub mod math;
pub mod pattern;
pub mod results;
pub mod revexpr;
pub mod search;
pub mod smc;

use clap::{Parser, ValueEnum};

pub use best_first::SearchPriority;

/// Which search algorithm to run.
#[derive(ValueEnum, Clone, Debug)]
pub enum SearchKind {
    /// Sequential Monte Carlo (stochastic particle filter).
    Smc,
    /// Best-first enumerative search over canonical patterns.
    BestFirst,
}

/// E-graph based program synthesis via SMC.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Search algorithm to use.
    #[arg(long, value_enum, default_value_t = SearchKind::Smc)]
    pub search: SearchKind,

    /// Path to the input JSON file containing programs.
    #[arg(short, long, default_value = "data/domains/cogsci/dials.json")]
    pub input: String,

    /// Path to rewrite rules file.
    #[arg(short, long)]
    pub rules: Option<String>,

    /// Follow pattern to constrain particle expansion.
    #[arg(short, long)]
    pub follow: Option<String>,

    /// Number of particles.
    #[arg(long, default_value_t = 10_000)]
    pub num_particles: usize,

    /// Number of SMC steps.
    #[arg(long, default_value_t = 1000)]
    pub num_steps: usize,

    /// Softmax temperature for resampling weights.
    #[arg(long, default_value_t = 100.0)]
    pub temperature: f64,

    /// Stop after this many steps with no improvement.
    #[arg(long, default_value_t = 50)]
    pub dead_runs: usize,

    /// Maximum arity of patterns to consider as "best".
    #[arg(long, default_value_t = 1000)]
    pub max_arity: usize,

    /// Heap priority for best-first search (only used when --search=best-first).
    #[arg(long, value_enum, default_value_t = SearchPriority::Cost)]
    pub priority: SearchPriority,

    /// Weight match selection by usage count during expansion.
    #[arg(long, default_value_t = false)]
    pub weight_by_usage: bool,

    /// Probability of attempting variable reuse during expansion.
    #[arg(long, default_value_t = 0.5)]
    pub p_reuse: f64,

    /// Enable slow rewrite check (assert fast == slow computation).
    #[arg(long, default_value_t = false)]
    pub check_slow: bool,

    /// Path to write JSON output.
    #[arg(short, long)]
    pub output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    pub debug_log: bool,

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}
