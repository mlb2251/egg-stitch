mod lang;
mod util;
mod pattern;
mod search;
mod revexpr;
mod smc;
mod rewrites;

use clap::Parser;

/// E-graph based program synthesis via SMC.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
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

    /// Weight match selection by usage count during expansion.
    #[arg(long, default_value_t = false)]
    pub weight_by_usage: bool,
}

fn main() {
    let args = Args::parse();

    let rules = args.rules.as_deref();
    let (egraph, root) = util::load_egraph(&args.input, rules);

    smc::smc(egraph, root, &args);
}
