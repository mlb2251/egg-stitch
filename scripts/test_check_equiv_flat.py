#!/usr/bin/env python3
"""Unit tests for `check_equiv.py`'s flat TypeScript dialect (`--flat`).

Run directly: `python3 scripts/test_check_equiv_flat.py`. Exits nonzero on any
failure. Kept tiny and dependency-free so CI can invoke it without pytest, the
same rule its sibling `test_check_equiv.py` follows.

Separate from `test_check_equiv.py` on purpose: that suite pins the *curried*
lambda-calc path every existing fixture runs through, so keeping it byte-identical
means a failure there is unambiguously a regression in the shared core rather than
fallout from the dialect work. This file covers only what `--flat` adds:
  - `lam{n}` binder groups desugaring to n nested unary binders, right-to-left
  - variadic `app` currying in source order, and `define` as a let
  - the dialect being opt-in — the default parser must NOT accept these heads
  - β through a TS-shaped library entry, end to end
  - the `--flat` flag reaching every parse site inside a whole `*.out.json` run
"""

import sys
import traceback

from check_equiv import (
    beta_normalize,
    build_library,
    inline_symbols,
    parse_term,
)


FAILS = []


def check(label, cond, detail=""):
    if cond:
        print(f"ok   {label}")
    else:
        # ASCII separator, not the em-dash `test_check_equiv.py` uses: this
        # console's cp1252 stdout can't encode em-dash, so a FAIL with a detail
        # string would raise UnicodeEncodeError and swallow the diagnostic
        # right when it's needed. Deliberate one-character divergence from the
        # sibling file, which otherwise stays byte-identical.
        print(f"FAIL {label}{(' - ' + detail) if detail else ''}")
        FAILS.append(label)


def test_flat_lam_group_desugars_to_nested_binders():
    """`lam{n}` desugars to exactly n nested unary `lam`s. This only pins the
    binder *count*, not which slot is outer vs inner: nested unary binders are
    symmetric — `("lam", ("lam", body))` is the same tuple regardless of which
    direction the slots are numbered — so flipping the numbering convention
    would leave this test green. `test_flat_beta_reduces_a_ts_shaped_abstraction`
    is the one that actually pins the direction, via a reduction whose result
    differs under the two conventions."""
    check("flat lam2 == two nested lams",
          parse_term("(lam2 $0)", flat=True) == parse_term("(lam (lam $0))"))
    check("flat lam1 == one lam",
          parse_term("(lam1 $0)", flat=True) == parse_term("(lam $0)"))
    check("flat lam3 == three nested lams",
          parse_term("(lam3 $2)", flat=True) == parse_term("(lam (lam (lam $2)))"))


def test_flat_app_curries_left_to_right():
    check("flat binary app",
          parse_term("(app f x)", flat=True) == parse_term("(@ f x)"))
    check("flat ternary app curries in source order",
          parse_term("(app f x y)", flat=True) == parse_term("(@ (@ f x) y)"))
    check("flat nullary-arg app is just the callee",
          parse_term("(app f)", flat=True) == parse_term("f"))


def test_flat_define_is_a_let_binding():
    """`TsOp::Define` binds its SECOND child, so `(define v body)` is
    `let x = v in body` — i.e. `((lam body) v)`. Getting the two children the
    wrong way round would type-check fine and check the wrong program."""
    check("define desugars to an applied lam",
          parse_term("(define v $0)", flat=True) == parse_term("(@ (lam $0) v)"))


def test_flat_dialect_is_opt_in():
    """Without `flat=True` the same source is silently misread: `lam2` and `app`
    fall through to the currying branch as ordinary symbols. That silence is why
    the dialect is selected explicitly and never inferred from a parse error."""
    default = parse_term("(lam2 (app f $0))")
    check("default parser applies lam2 as an opaque symbol",
          default == ("app", ("sym", "lam2"), parse_term("(app f $0)")), repr(default))
    check("default parse differs from flat parse",
          default != parse_term("(lam2 (app f $0))", flat=True))


def test_flat_beta_reduces_a_ts_shaped_abstraction():
    """End-to-end on the shape Task 3's fixture produces: a `lam2`-bodied
    library entry applied to two arguments must β-reduce back to the inlined
    call, exactly as the curried lambda-calc equivalent does.

    This is the whole soundness argument for the flat dialect's slot numbering:
    `(app fn_0 a b)` against `(lam2 (app P $0 $1))` must reduce to
    `(app P b a)`, not `(app P a b)`. Unlike the nested-binder shape alone
    (symmetric under a flipped convention), this assertion fails if slot 0
    ever stops being the outer binder — so if the family's numbering
    convention is ever flipped, this test is what catches it."""
    lib = build_library([{"pattern": "fn_0", "lambda": "(lam2 (app P $0 $1))"}], flat=True)
    rewr = inline_symbols(parse_term("(app fn_0 a b)", flat=True), lib)
    rewr_nf, left = beta_normalize(rewr, 100)
    check("ts-shaped call reduces to the substituted body",
          left > 0 and rewr_nf == parse_term("(app P b a)", flat=True), repr(rewr_nf))


def test_flat_flag_drives_a_whole_run_result():
    """The plumbing test: a TS-shaped `*.out.json` must check out under
    `flat=True` and must NOT be quietly accepted under the default dialect,
    where `lam2`/`app` degrade to symbols."""
    import argparse
    import json
    import os
    import tempfile

    from check_equiv import check_file

    run = {
        "original_programs": ["(app F e (lam2 (app P $0 (app g $1))))"],
        "rewritten_programs": ["(app F e (app fn_0 g))"],
        "library": [{"pattern": "fn_0", "lambda": "(lam1 (lam2 (app P $0 (app $2 $1))))"}],
    }
    fd, path = tempfile.mkstemp(suffix=".out.json")
    with os.fdopen(fd, "w") as f:
        json.dump(run, f)
    try:
        flat_args = argparse.Namespace(rewrites=None, fuel=100_000, iters=30, nodes=10_000, verbose=False, flat=True)
        check("flat dialect proves the ts-shaped run equivalent", check_file(path, flat_args))
        default_args = argparse.Namespace(rewrites=None, fuel=100_000, iters=30, nodes=10_000, verbose=False, flat=False)
        check("default dialect does not prove it", not check_file(path, default_args))
    finally:
        os.unlink(path)


def main():
    tests = [
        test_flat_lam_group_desugars_to_nested_binders,
        test_flat_app_curries_left_to_right,
        test_flat_define_is_a_let_binding,
        test_flat_dialect_is_opt_in,
        test_flat_beta_reduces_a_ts_shaped_abstraction,
        test_flat_flag_drives_a_whole_run_result,
    ]
    for t in tests:
        try:
            t()
        except Exception:
            FAILS.append(getattr(t, "__name__", repr(t)))
            traceback.print_exc()

    print()
    if FAILS:
        print(f"{len(FAILS)} failure(s): {FAILS}")
        sys.exit(1)
    print("all tests passed")


if __name__ == "__main__":
    main()
