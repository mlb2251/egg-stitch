mod cost;
mod io;
mod lang;
mod matching;
mod pattern;
mod revexpr;
mod search;
mod smc;

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

    /// Probability of attempting variable reuse during expansion.
    #[arg(long, default_value_t = 0.5)]
    pub p_reuse: f64,
}

fn main() {
    let args = Args::parse();
    let (egraph, root) = io::load_egraph(&args.input, args.rules.as_deref());
    smc::smc(egraph, root, &args);
}
