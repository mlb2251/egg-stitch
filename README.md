

> **Note:** The experiment runner (`expts/`, `run.py`) and visualization tools (`viz/`) were vibe-coded and have not been carefully reviewed. Use at your own risk.

# Key commands


Launch results visualization server (if not already running) with:
```
make server
```

View experiments at [http://localhost:8066/viz/](http://localhost:8066/viz/).


run all experiments
```
python3 -c 'from expts import *; runall(num_steps=10, num_particles=100)'
```

Debug the dials domain

```
python3 -c 'from expts import *; run_domain("dials", num_steps=10, num_particles=1000, debug_log=True)'
```

run a single (method, domain) cell of Table 1/2 directly

```
python3 -c 'from expts import *; print(run_method(OursSmc(), "dials", rounds=1, use_dsrs=True)[0].summary_line())'
```

Each runner (`OursSmc`, `OursBf`, `Babble`, `Stitch`) is a frozen
dataclass carrying its own hyperparameters as fields — pass overrides as
kwargs at construction (e.g. `OursSmc(num_steps=50)`) instead of mutating
module state. To invoke the egg-stitch binary
directly with custom flags, drive ``$(cargo build --release && ls
target/release/egg-stitch)`` yourself.




