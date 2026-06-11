#!/usr/bin/env python3
"""Pointed unit tests for `check_equiv.py`.

Run directly: `python3 scripts/test_check_equiv.py`. Exits nonzero on any
failure. Kept tiny and dependency-free so CI can invoke it without pytest.

Covers the bits most likely to silently degrade:
  - de Bruijn shift/subst correctness across binders
  - the HO-arity-2 capture case from `tests/stitch_compat_test.rs`
    (`($0 $1)` vs `($1 $0)` under a wrap — must NOT be β-equivalent)
  - library inlining + β-equivalence end-to-end
  - DSR-mediated equivalence via the e-graph mode (`(* 0 ?x) <=> 0`)
  - `constant_folding` directive folding across kinds (`!numbers` / `!floats` /
    `!integersarefloats`), composition with DSRs, and non-overreach
  - mismatch detection (the oracle must refuse to call non-equivalents equal)
"""

import sys
import traceback

from check_equiv import (
    beta_contract,
    beta_normalize,
    build_library,
    equiv_under_rules,
    inline_symbols,
    parse_rewrites,
    parse_term,
    shift,
    subst,
)


FAILS = []


def check(label, cond, detail=""):
    if cond:
        print(f"ok   {label}")
    else:
        print(f"FAIL {label}{(' — ' + detail) if detail else ''}")
        FAILS.append(label)


def nf(s, fuel=10_000):
    t, left = beta_normalize(parse_term(s), fuel)
    assert left > 0, f"β fuel exhausted on {s!r}"
    return t


# --- shift / subst sanity --------------------------------------------------

def test_shift_respects_cutoff():
    # $0 below the cutoff is untouched; $1 at-or-above is bumped.
    t = parse_term("(lam (@ $0 $1))")  # under the outer lam, body is (@ $0 $1)
    # shift the whole term by +1 starting at cutoff 0: $1 becomes $2 inside body.
    shifted = shift(t, 1, 0)
    check("shift bumps free vars only",
          shifted == ("lam", ("app", ("var", 0), ("var", 2))),
          repr(shifted))


def test_subst_under_binder():
    # ((lam (lam $1)) y)  β→  (lam y)   — the inner $1 refers to the OUTER
    # binder, which the redex consumes; y must end up unshifted inside the
    # surviving lam.
    body = parse_term("(lam $1)")   # inside the outer lam we just stripped
    arg = parse_term("y")
    out = beta_contract(body, arg)
    check("β across an inner binder yields (lam y)",
          out == ("lam", ("sym", "y")),
          repr(out))


# --- β-normalization on whole terms ---------------------------------------

def test_identity_application():
    check("(lam $0) y  ≡  y",
          nf("(@ (lam $0) y)") == parse_term("y"))


def test_K_combinator():
    # K x y = x
    check("K x y ≡ x",
          nf("(@ (@ (lam (lam $1)) x) y)") == parse_term("x"))


def test_argument_order_matters():
    # The exact bug the test-comment in stitch_compat_test.rs flagged: under
    # a (lam (lam …)) wrap, swapping the application order is observable.
    # ($0 $1) vs ($1 $0) inside two binders must NOT be β-equivalent.
    a = nf("(lam (lam (@ $0 $1)))")
    b = nf("(lam (lam (@ $1 $0)))")
    check("HO-arity-2 swap is detected as non-equivalent",
          a != b, f"\n  a={a}\n  b={b}")


# --- library inlining ------------------------------------------------------

def test_library_inlining_then_beta():
    # A one-entry library: `f` is the identity. The "rewritten" program calls
    # `f` on `y`; after inlining + β it must equal `y`.
    lib = build_library([{"pattern": "f", "lambda": "(lam $0)"}])
    rewr = inline_symbols(parse_term("(@ f y)"), lib)
    rewr_nf, _ = beta_normalize(rewr, 100)
    check("library `f := λx.x`; (f y) inlines+β to y",
          rewr_nf == parse_term("y"), repr(rewr_nf))


def test_library_chained_entries():
    # Second entry references the first; build_library must inline the prior
    # entry into the later one's body so the final lib has no library refs.
    lib = build_library([
        {"pattern": "id", "lambda": "(lam $0)"},
        {"pattern": "twice", "lambda": "(lam (@ id (@ id $0)))"},
    ])
    rewr_nf, _ = beta_normalize(inline_symbols(parse_term("(@ twice y)"), lib), 100)
    check("chained library `twice` reduces to y",
          rewr_nf == parse_term("y"), repr(rewr_nf))


# --- e-graph mode with DSRs ------------------------------------------------

def test_dsr_annihilator(tmp_rules):
    # `(* 0 ?x) => 0` ; under this DSR, (* 0 y) and 0 must unify.
    # Uses `=>` (not `<=>`) to mirror the real `*.rewrites` files in
    # `data/domains/`, where the pvar-asymmetric direction is omitted.
    rules = parse_rewrites(tmp_rules)
    same, status = equiv_under_rules(
        parse_term("(* 0 y)"), parse_term("0"), rules, max_iters=20, max_nodes=2_000,
    )
    check("DSR (* 0 ?x) => 0 unifies (* 0 y) with 0", same, status)


def test_dsr_does_not_overreach(tmp_rules):
    # Same rule set must NOT equate (* 0 y) with (* 1 y).
    rules = parse_rewrites(tmp_rules)
    same, status = equiv_under_rules(
        parse_term("(* 0 y)"), parse_term("(* 1 y)"), rules, max_iters=20, max_nodes=2_000,
    )
    check("DSR does not collapse (* 0 y) with (* 1 y)", not same, status)


# --- e-graph mode with constant folding ------------------------------------

def test_constant_folding_unifies(fold_rules):
    # `constant_folding: !numbers` folds (+ 1 2) into 3, so they must unify.
    rules = parse_rewrites(fold_rules)
    same, status = equiv_under_rules(
        parse_term("(+ 1 2)"), parse_term("3"), rules, max_iters=20, max_nodes=2_000,
    )
    check("constant folding unifies (+ 1 2) with 3", same, status)


def test_constant_folding_nested(fold_rules):
    # Folding composes bottom-up: (* (+ 1 2) (- 10 4)) == 18.
    rules = parse_rewrites(fold_rules)
    same, status = equiv_under_rules(
        parse_term("(* (+ 1 2) (- 10 4))"), parse_term("18"), rules, max_iters=20, max_nodes=2_000,
    )
    check("constant folding folds nested arithmetic to 18", same, status)


def test_constant_folding_no_overreach(fold_rules):
    # Folding must not equate distinct values: (+ 1 2) != 4.
    rules = parse_rewrites(fold_rules)
    same, status = equiv_under_rules(
        parse_term("(+ 1 2)"), parse_term("4"), rules, max_iters=20, max_nodes=2_000,
    )
    check("constant folding does not equate (+ 1 2) with 4", not same, status)


def test_constant_folding_skips_inexact(fold_rules):
    # Inexact integer division is left unfolded, mirroring the Rust path, so
    # (/ 7 2) must not collapse to 3.
    rules = parse_rewrites(fold_rules)
    same, status = equiv_under_rules(
        parse_term("(/ 7 2)"), parse_term("3"), rules, max_iters=20, max_nodes=2_000,
    )
    check("constant folding leaves inexact (/ 7 2) unfolded", not same, status)


def test_constant_folding_after_dsr(chain_rules):
    # Folding fires only after non-minimizing rewrites rearrange the term:
    # (* 2 tau) =tau=> (* 2 (* 2 pi)) =assoc=> (* (* 2 2) pi) =fold=> (* 4 pi).
    # The folded subterm (* 2 2) is in neither side; it appears mid-saturation.
    # Mirrors the `constant_folding_after_rewrites` stitch_compat fixture.
    rules = parse_rewrites(chain_rules)
    same, status = equiv_under_rules(
        parse_term("(* 2 tau)"), parse_term("(* 4 pi)"), rules, max_iters=20, max_nodes=5_000,
    )
    check("folding composes with tau-def + assoc: (* 2 tau) ≡ (* 4 pi)", same, status)


def _equiv(a, b, rules_path):
    return equiv_under_rules(parse_term(a), parse_term(b), parse_rewrites(rules_path), max_iters=20, max_nodes=2_000)[0]


def test_integers_mode(integers_rules):
    # !integers folds integer-only operations but leaves float ones alone.
    check("!integers folds (+ 1 2) ≡ 3", _equiv("(+ 1 2)", "3", integers_rules))
    check("!integers leaves (+ 1.5 2.5) unfolded vs 4.0", not _equiv("(+ 1.5 2.5)", "4.0", integers_rules))


def test_floats_mode(floats_rules):
    # !floats folds float / mixed operations but leaves integer-only ones alone.
    check("!floats folds (+ 1.5 2.5) ≡ 4.0", _equiv("(+ 1.5 2.5)", "4.0", floats_rules))
    check("!floats leaves (+ 1 2) unfolded vs 3", not _equiv("(+ 1 2)", "3", floats_rules))


def test_integers_are_floats_mode(iaf_rules):
    # !integersarefloats reads ints as floats, so it folds integer ops in float
    # and unifies each integer literal with its float form.
    check("!integersarefloats folds (+ 1 2) ≡ 3.0", _equiv("(+ 1 2)", "3.0", iaf_rules))
    check("!integersarefloats unifies 4 ≡ 4.0", _equiv("4", "4.0", iaf_rules))


def test_folding_alone_keeps_int_float_distinct(fold_rules):
    # Plain folding (no `n => n.0`) does not equate the literals 4 and 4.0.
    check("!numbers keeps 4 and 4.0 distinct", not _equiv("4", "4.0", fold_rules))


# --- driver ----------------------------------------------------------------

def main():
    import tempfile, os
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("annihilator: (* 0 ?x) => 0\n")
        rules_path = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("constant_folding: !numbers\n")
        fold_path = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("tau_def: tau <=> (* 2 pi)\n")
        f.write("mult_assoc: (* ?a (* ?b ?c)) <=> (* (* ?a ?b) ?c)\n")
        f.write("constant_folding: !numbers\n")
        chain_path = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("constant_folding: !integers\n")
        integers_path = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("constant_folding: !floats\n")
        floats_path = f.name
    with tempfile.NamedTemporaryFile("w", suffix=".rewrites", delete=False) as f:
        f.write("constant_folding: !integersarefloats\n")
        iaf_path = f.name
    try:
        tests = [
            test_shift_respects_cutoff,
            test_subst_under_binder,
            test_identity_application,
            test_K_combinator,
            test_argument_order_matters,
            test_library_inlining_then_beta,
            test_library_chained_entries,
            lambda: test_dsr_annihilator(rules_path),
            lambda: test_dsr_does_not_overreach(rules_path),
            lambda: test_constant_folding_unifies(fold_path),
            lambda: test_constant_folding_nested(fold_path),
            lambda: test_constant_folding_no_overreach(fold_path),
            lambda: test_constant_folding_skips_inexact(fold_path),
            lambda: test_constant_folding_after_dsr(chain_path),
            lambda: test_integers_mode(integers_path),
            lambda: test_floats_mode(floats_path),
            lambda: test_integers_are_floats_mode(iaf_path),
            lambda: test_folding_alone_keeps_int_float_distinct(fold_path),
        ]
        for t in tests:
            try:
                t()
            except Exception:
                FAILS.append(getattr(t, "__name__", repr(t)))
                traceback.print_exc()
    finally:
        for p in (rules_path, fold_path, chain_path, integers_path, floats_path, iaf_path):
            os.unlink(p)

    print()
    if FAILS:
        print(f"{len(FAILS)} failure(s): {FAILS}")
        sys.exit(1)
    print("all tests passed")


if __name__ == "__main__":
    main()
