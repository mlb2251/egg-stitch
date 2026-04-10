

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
python3 -c 'from expts import *; compress("data/domains/cogsci/dials.json", rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites", num_steps=100, num_particles=1000, debug_log=False, max_arity=2, temperature=1000)'
```

Follow:

```
python3 -c 'from expts import *; compress("data/domains/cogsci/dials.json", rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites", num_steps=10, num_particles=100, debug_log=False, follow="(T (T (T l (M 1 0 -0.5 0)) (M #0 (/ pi 4) 0 0)) (M 1 0 (* #0 (* 0.5 (cos (/ pi 4)))) (* #0 (* 0.5 (sin (/ pi 4))))))")'
```




