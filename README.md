

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

compress a json directly

```
python3 -c 'from expts import *; compress("data/domains/cogsci/dials.json", rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites", num_steps=10, num_particles=100, debug_log=False)'
```



