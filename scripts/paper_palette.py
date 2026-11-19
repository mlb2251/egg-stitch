"""The paper's color palette, shared with its TikZ figures (e-stitch-paper/commands.tex).

Every color in a figure is one of these bases, or a base mixed with white
(``tint``) or black (``shade``), matching xcolor's ``base!pct`` and
``base!pct!black``.
"""

BLUE = "#2f6fb0"
ORANGE = "#f0a860"
GREEN = "#1e8449"
PINK = "#d6457f"
PURPLE = "#7b52ab"
TEAL = "#0b8fa1"
BLACK = "#000000"


def _rgb(color: str) -> tuple[float, float, float]:
    return tuple(int(color[i:i + 2], 16) / 255 for i in (1, 3, 5))


def tint(color: str, pct: float) -> tuple[float, float, float]:
    """xcolor ``color!pct``: ``pct`` percent of ``color``, the rest white."""
    return tuple(c * pct / 100 + (1 - pct / 100) for c in _rgb(color))


def shade(color: str, pct: float) -> tuple[float, float, float]:
    """xcolor ``color!pct!black``: ``pct`` percent of ``color``, the rest black."""
    return tuple(c * pct / 100 for c in _rgb(color))


# Molecule diagrams, by atomic number (the paper's atomC/atomH/atomO).
ATOM_COLORS = {
    1: tint(BLACK, 67),
    6: _rgb(BLACK),
    8: _rgb(PINK),
}

# Line-strength versions; orange is darkened exactly as in the paper's
# search-tree figure (badgeorange!85!black).
METHOD_COLORS = {
    "enum": _rgb(BLUE),
    "smc": shade(ORANGE, 85),
    "babble": _rgb(GREEN),
    "stitch": _rgb(PINK),
    "enum-dsrs-at-start": _rgb(PURPLE),
    "enum-baseline": _rgb(BLACK),
}
