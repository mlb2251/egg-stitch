#!/usr/bin/env python3
"""Run experiment variants and collect results into viz/results/<timestamp>/.

Folder management lives in :mod:`expts.folders`; per-tool subprocess wrappers
in :mod:`expts.run_models` (each tool also owns its own binary path and
clean-main check); domain dispatch + multi-file aggregation in
:mod:`expts.runner`. ``from expts import *`` re-exports everything the table
runners need.
"""

from .folders import *


# Cogsci domains (single-file inputs under data/domains/cogsci/).
COGSCI_DOMAINS = ["dials", "furniture", "nuts-bolts", "wheels"]
# Dreamcoder benchmark domains (multi-file directories under
# data/domains/<name>/, generated via babble's ``benchmark --dump`` mode).
# Programs are in lambda-calc form; only ``list`` and ``physics`` ship with
# an equational theory in babble (text/logo/towers run without DSRs,
# matching Fig. 12 in the babble paper).
DREAMCODER_DOMAINS = ["list", "physics", "text", "logo", "towers"]
ALL_DOMAINS = COGSCI_DOMAINS + DREAMCODER_DOMAINS


# Pulled in at the bottom so submodules can import the constants/helpers
# defined above. ``from expts import *`` then re-exports everything.
from .runner import *
from .bench import *
from .run_models import *
from .tables import *
