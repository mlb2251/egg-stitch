

> **Note:** The experiment runner (`expts/`, `run.py`) and visualization tools (`viz/`) were vibe-coded and have not been carefully reviewed. Use at your own risk.

# Key commands


Install the Python dependencies for the experiment/plotting scripts with:
```
pip install -r requirements.txt
```

Launch results visualization server (if not already running) with:
```
make server
```

View experiments at [http://localhost:8066/viz/](http://localhost:8066/viz/).


run all experiments
```
python3 -c 'from expts import *; runall(num_steps=10, num_particles=100)'
```

Each runner (`OursSmc`, `OursBf`, `Babble`, `Stitch`) is a frozen
dataclass carrying its own hyperparameters as fields — pass overrides as
kwargs at construction (e.g. `OursSmc(num_steps=50)`) instead of mutating
module state. To invoke the egg-stitch binary
directly with custom flags, drive ``$(cargo build --release && ls
target/release/egg-stitch)`` yourself.




