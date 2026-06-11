#!/usr/bin/env python3
"""Convert acyclic molecules into egg-stitch tree S-expressions + symmetry rewrites.

Representation: each atom is `(<head> <Element> <neighbour> ...)` where
  head = <parent-bond><degree>: parent-bond is `m` for the molecule root (no
  parent) or `s`/`d`/`t` for a single/double/triple bond up to the parent, and
  <degree> is the atom's neighbour count. The element symbol sits in the first
  child slot (so a rewrite metavariable can bind it); the remaining children are
  the atom's neighbours other than its parent. Valence is therefore implicit
  (sum of a node's bond orders) while degree (arity) is explicit.

Only single/double/triple bonds are handled and molecules must be acyclic, so the
molecular graph is a tree. Structures come from PubChem: a recognisable seed set
fetched by name, then filled out by scanning a range of PubChem CIDs. Everything
is cached in scripts/molecule_smiles.json so regeneration needs no network.

Emits:
  data/domains/molecules/molecules.json      the corpus (one rooted tree each)
  data/domains/molecules/molecules.rewrites  re-rooting + commutativity rules

Usage: python3 scripts/mol_to_tree.py
"""
import json
import os
import sys
import time
import urllib.parse
import urllib.request

from rdkit import Chem
from rdkit import RDLogger

RDLogger.DisableLog("rdApp.*")

HERE = os.path.dirname(os.path.abspath(__file__))   # scripts/molecules
ROOT = os.path.dirname(os.path.dirname(HERE))        # project root
CACHE = os.path.join(HERE, "molecule_smiles.json")

# Tunables for the bulk scan.
TARGET = 5000         # stop once we have this many distinct molecules
MAX_HEAVY = 10        # cap heavy-atom count so trees stay small
MAX_CID = 80000       # how far up the PubChem CID range to scan
BATCH = 200           # CIDs per PubChem request
ALLOWED = {"H", "C", "N", "O", "F", "Cl", "Br", "I", "S", "P"}

# A recognisable seed set, fetched by name, so the corpus opens with familiar
# molecules across the common functional groups.
NAMES = [
    "methane", "ethane", "propane", "butane", "isobutane", "pentane",
    "neopentane", "hexane", "ethylene", "propene", "1-butene", "isobutylene",
    "1,3-butadiene", "acetylene", "propyne", "methanol", "ethanol", "1-propanol",
    "isopropanol", "ethylene glycol", "glycerol", "dimethyl ether",
    "diethyl ether", "formaldehyde", "acetaldehyde", "propionaldehyde",
    "acetone", "2-butanone", "formic acid", "acetic acid", "propionic acid",
    "methyl acetate", "ethyl acetate", "methyl formate", "methylamine",
    "dimethylamine", "trimethylamine", "ethylamine", "formamide", "acetamide",
    "urea", "hydrogen cyanide", "acetonitrile", "propionitrile", "water",
    "ammonia", "carbon dioxide", "hydrogen peroxide",
]

BOND_MARK = {1: "s", 2: "d", 3: "t"}  # integer bond order -> parent-bond marker


def http_get(url):
    """GET a URL, returning decoded text or None on failure."""
    try:
        with urllib.request.urlopen(url, timeout=30) as resp:
            return resp.read().decode()
    except Exception as exc:
        print(f"  ! request failed: {exc}", file=sys.stderr)
        return None


def fetch_named(cache):
    """Fetch SMILES for the seed NAMES from PubChem (cached)."""
    smiles = []
    for name in NAMES:
        key = f"name:{name}"
        if key not in cache:
            txt = http_get(
                "https://pubchem.ncbi.nlm.nih.gov/rest/pug/compound/name/"
                + urllib.parse.quote(name)
                + "/property/CanonicalSMILES/TXT"
            )
            cache[key] = txt.strip().splitlines()[0].strip() if txt else None
            time.sleep(0.2)
        if cache[key]:
            smiles.append(cache[key])
    return smiles


def fetch_cid_batch(cids, cache):
    """Fetch SMILES for a list of CIDs from PubChem (cached), in CID order."""
    missing = [c for c in cids if f"cid:{c}" not in cache]
    if missing:
        ids = ",".join(str(c) for c in missing)
        txt = http_get(
            "https://pubchem.ncbi.nlm.nih.gov/rest/pug/compound/cid/"
            + ids
            + "/property/CanonicalSMILES/CSV"
        )
        got = {}
        if txt:
            for line in txt.splitlines()[1:]:  # skip CSV header
                parts = line.split(",")
                if len(parts) >= 2:
                    cid = parts[0].strip().strip('"')
                    smi = parts[1].strip().strip('"')
                    if cid.isdigit() and smi:
                        got[int(cid)] = smi
        for c in missing:
            cache[f"cid:{c}"] = got.get(c)  # None records a confirmed miss
        time.sleep(0.2)
    return [cache[f"cid:{c}"] for c in cids if cache.get(f"cid:{c}")]


def prepare_mol(smiles):
    """Parse SMILES into an acyclic, neutral, in-scope mol with explicit Hs.

    Returns the prepared mol or None if it cannot be represented as an acyclic
    single/double/triple-bonded tree over the allowed elements.
    """
    if not smiles or "." in smiles:  # reject mixtures / salts
        return None
    mol = Chem.MolFromSmiles(smiles)
    if mol is None:
        return None
    if mol.GetRingInfo().NumRings() > 0:
        return None
    if any(b.GetIsAromatic() for b in mol.GetBonds()):
        return None
    heavy = 0
    for atom in mol.GetAtoms():
        if atom.GetSymbol() not in ALLOWED:
            return None
        if atom.GetFormalCharge() != 0 or atom.GetIsotope() != 0:
            return None
        if atom.GetAtomicNum() > 1:
            heavy += 1
    if not 1 <= heavy <= MAX_HEAVY:
        return None
    return Chem.AddHs(mol)


def canonical(mol):
    """Canonical SMILES (no explicit Hs) for de-duplication."""
    return Chem.MolToSmiles(Chem.RemoveHs(mol))


def bond_order(bond):
    """Integer bond order (1/2/3) for a non-aromatic bond."""
    return int(round(bond.GetBondTypeAsDouble()))


def root_atom(mol):
    """Pick a deterministic heavy-atom root (lowest canonical rank)."""
    ranks = list(Chem.CanonicalRankAtoms(mol, includeChirality=True))
    heavy = [a.GetIdx() for a in mol.GetAtoms() if a.GetAtomicNum() > 1]
    candidates = heavy or [a.GetIdx() for a in mol.GetAtoms()]
    return min(candidates, key=lambda i: ranks[i])


def node_sexpr(mol, idx, parent, parent_order):
    """Render the subtree rooted at atom `idx` (reached from `parent`)."""
    atom = mol.GetAtomWithIdx(idx)
    mark = "m" if parent is None else BOND_MARK[parent_order]
    head = f"{mark}{atom.GetDegree()}"
    children = [atom.GetSymbol()]
    for bond in atom.GetBonds():
        nbr = bond.GetOtherAtomIdx(idx)
        if nbr == parent:
            continue
        children.append(node_sexpr(mol, nbr, idx, bond_order(bond)))
    return "(" + " ".join([head, *children]) + ")"


def to_tree(mol):
    """Convert a prepared mol into its rooted tree S-expression."""
    return node_sexpr(mol, root_atom(mol), None, None)


def collect_shapes(mol, bond_types, sp3_degrees, term_multibond=None):
    """Record this molecule's bond types, all-single-bond atom degrees, and the
    `(order, degree)` shapes of multi-bonded centres whose multi-bond partner is
    terminal (degree 1), e.g. a carbonyl `C=O` -> `(2, 3)`."""
    for bond in mol.GetBonds():
        di, dj = bond.GetBeginAtom().GetDegree(), bond.GetEndAtom().GetDegree()
        bond_types.add((min(di, dj), bond_order(bond), max(di, dj)))
        if term_multibond is not None and bond_order(bond) >= 2:
            for end, other in ((bond.GetBeginAtom(), bond.GetEndAtom()), (bond.GetEndAtom(), bond.GetBeginAtom())):
                # `end` is the centre (>=2 substituents to reorder), `other` the
                # terminal partner across the multi-bond.
                if other.GetDegree() == 1 and end.GetDegree() >= 3:
                    term_multibond.add((bond_order(bond), end.GetDegree()))
    for atom in mol.GetAtoms():
        if all(bond_order(b) == 1 for b in atom.GetBonds()):
            sp3_degrees.add(atom.GetDegree())


def sexpr(head, elem, neighbours):
    """Assemble `(head elem n1 n2 ...)`."""
    return "(" + " ".join([head, elem, *neighbours]) + ")"


def reroot_rule(i, order, j):
    """Bidirectional rule moving the root across an (i)-(order)-(j) bond."""
    mark = BOND_MARK[order]
    xs = [f"?x{k}" for k in range(i - 1)]
    ys = [f"?y{k}" for k in range(j - 1)]
    lhs = sexpr(f"m{i}", "?e", [*xs, sexpr(f"{mark}{j}", "?f", ys)])
    rhs = sexpr(f"m{j}", "?f", [*ys, sexpr(f"{mark}{i}", "?e", xs)])
    return f"reroot_{mark}_{i}_{j}: {lhs} <=> {rhs}"


def perm_rules(head, n, mode):
    """Element-generic commutativity over the `n` neighbour slots of `head`.

    Every permutation emitted is a proper rotation of the local geometry, so it
    maps the molecule to itself for *any* element and can never merge a
    stereoisomer: 'A4' is the tetrahedral rotation group (n==4), 'C3' a cyclic
    rotation (n==3), 'swap' the C2 of a divalent centre (n==2).
    """
    base = [f"?n{k}" for k in range(n)]

    def rule(name, perm):
        return f"{name}: {sexpr(head, '?e', base)} => {sexpr(head, '?e', [base[p] for p in perm])}"

    if mode == "A4":  # two 3-cycles (slots 0-1-2 and 1-2-3) generate A4
        return [
            rule(f"rot_{head}_a", [1, 2, 0, 3]),
            rule(f"rot_{head}_b", [0, 2, 3, 1]),
        ]
    if mode == "C3":
        return [rule(f"rot_{head}", [(k + 1) % n for k in range(n)])]
    if mode == "swap":
        return [rule(f"comm_{head}", [1, 0])]
    return []


def commutativity_rules(sp3_degrees):
    """Proper-rotation commutativity for every all-single-bond atom shape.

    A shape appears as a root (`m<deg>`, all `deg` neighbours shown) and, via
    re-rooting, as a single-bonded child (`s<deg>`, parent pinned so `deg-1`
    shown). The pinned parent removes the rotations that move it, so e.g. a
    tetrahedral centre drops from A4 (root) to C3 (child), and a pyramidal centre
    keeps no non-trivial rotation as a child.
    """
    root_mode = {4: "A4", 3: "C3", 2: "swap"}
    child_mode = {4: "C3"}  # only the tetrahedral child keeps a rotation
    out = []
    for deg in sorted(sp3_degrees):
        if deg in root_mode:
            out.extend(perm_rules(f"m{deg}", deg, root_mode[deg]))
        if deg in child_mode:
            out.extend(perm_rules(f"s{deg}", deg - 1, child_mode[deg]))
    return out


def dup_swap_rule(head, n):
    """A transposition of the first two of `head`'s `n` shown slots, guarded by
    the last two slots being the *same* metavariable.

    Firing requires two substituents to be identical, which means the centre is
    not a stereocentre, so the swap of the other two cannot fabricate an
    enantiomer -- sound for any element. Together with the proper rotations above
    (which bring any repeated pair into the last two slots) this realises the
    full neighbour-permutation group at every centre that has a repeated
    substituent (e.g. every CH2 / CH3 carbon).
    """
    slots = [f"?n{k}" for k in range(n - 1)] + [f"?n{n - 2}"]  # last pair shared
    swapped = [slots[1], slots[0], *slots[2:]]
    return f"comm_{head}_dup: {sexpr(head, '?e', slots)} => {sexpr(head, '?e', swapped)}"


def dup_swap_rules(sp3_degrees):
    """Duplicate-guarded transpositions for every all-single-bond shape with at
    least three shown slots (root `m>=3`, single-bonded child `s>=4`)."""
    out = []
    for deg in sorted(sp3_degrees):
        if deg >= 3:
            out.append(dup_swap_rule(f"m{deg}", deg))
        if deg - 1 >= 3:
            out.append(dup_swap_rule(f"s{deg}", deg - 1))
    return out


def terminal_multibond_rules(term_multibond):
    """Free commutativity for a multi-bonded centre whose multi-bond partner is
    terminal -- e.g. a carbonyl, where the `=O` has no second substituent so no
    E/Z geometry exists and the centre's remaining neighbours reorder freely.

    Emitted in the rooted-at-the-terminal-partner form `(m1 ?p (<order><deg> ...))`;
    re-rooting carries it to such centres anywhere in the tree. Adjacent
    transpositions of the centre's `deg-1` children generate their full symmetric
    group. Sound for any element because a degree-1 partner cannot bear the
    second substituent E/Z would require.
    """
    out = []
    for order, deg in sorted(term_multibond):
        mark = BOND_MARK[order]
        kids = [f"?n{k}" for k in range(deg - 1)]
        for k in range(deg - 2):
            swapped = kids[:]
            swapped[k], swapped[k + 1] = swapped[k + 1], swapped[k]
            inner_l = sexpr(f"{mark}{deg}", "?c", kids)
            inner_r = sexpr(f"{mark}{deg}", "?c", swapped)
            out.append(f"comm_term_{mark}{deg}_{k}: {sexpr('m1', '?p', [inner_l])} => {sexpr('m1', '?p', [inner_r])}")
    return out


def main():
    cache = {}
    if os.path.exists(CACHE):
        with open(CACHE) as fh:
            cache = json.load(fh)

    seen, mols = set(), []

    def add(smiles):
        mol = prepare_mol(smiles)
        if mol is None:
            return
        key = canonical(mol)
        if key in seen:
            return
        seen.add(key)
        mols.append(mol)

    for smi in fetch_named(cache):
        add(smi)

    cid = 1
    while len(mols) < TARGET and cid <= MAX_CID:
        batch = list(range(cid, min(cid + BATCH, MAX_CID + 1)))
        for smi in fetch_cid_batch(batch, cache):
            add(smi)
        cid += BATCH
        print(f"  scanned CIDs up to {cid - 1}: {len(mols)} molecules", file=sys.stderr)

    with open(CACHE, "w") as fh:
        json.dump(cache, fh, indent=2, sort_keys=True)
        fh.write("\n")

    trees = [to_tree(mol) for mol in mols]
    bond_types, sp3_degrees, term_multibond = set(), set(), set()
    for mol in mols:
        collect_shapes(mol, bond_types, sp3_degrees, term_multibond)

    reroots = [reroot_rule(i, o, j) for (i, o, j) in sorted(bond_types)]
    comms = commutativity_rules(sp3_degrees)
    dups = dup_swap_rules(sp3_degrees)
    terms = terminal_multibond_rules(term_multibond)

    outdir = os.path.join(ROOT, "data/domains/molecules")
    os.makedirs(outdir, exist_ok=True)
    with open(os.path.join(outdir, "molecules.json"), "w") as fh:
        json.dump(trees, fh, indent=2)
        fh.write("\n")

    header = [
        "// AUTO-GENERATED by scripts/mol_to_tree.py -- do not edit by hand.",
        "//",
        "// Atom node `(<head> E n1 n2 ...)`: head = <parent-bond><degree> where the",
        "// parent-bond is `m` (root), or `s`/`d`/`t` for a single/double/triple bond up",
        "// to the parent, and degree is the neighbour count. The element E sits in the",
        "// first child slot so a metavariable can bind it; the rest are the neighbours.",
        "//",
        "// Re-rooting (element-generic: relabelling the root preserves the graph and",
        "// every bond order, so it is stereochemically safe):",
    ]
    lines = header + reroots + [
        "",
        "// Commutativity (neighbours are unordered). Unconditionally only proper",
        "// rotations of the local geometry, so no rule ever merges a stereoisomer:",
        "//   * a double/triple-bonded centre gets no rotation (a swap would flip E/Z);",
        "//   * a tetrahedral centre uses A4 as a root, C3 with the parent pinned (never a",
        "//     bare transposition, so enantiomers stay distinct);",
        "//   * a pyramidal centre uses C3 as a root and nothing as a child; a divalent",
        "//     centre uses its C2 swap.",
    ] + comms + [
        "",
        "// Transpositions guarded by a local achirality witness -- always sound, and",
        "// together with the rotations above they close the full neighbour-permutation",
        "// group on the achiral centres a random scramble can produce:",
        "//   * `_dup`: two substituents identical => centre is not stereogenic, so the",
        "//     other two may be swapped (the shared `?n` slot is the witness);",
    ] + dups + [
        "//   * `_term`: the multi-bond partner is terminal (`m1`, e.g. a carbonyl =O),",
        "//     so no E/Z geometry exists and the centre's neighbours reorder freely.",
    ] + terms

    with open(os.path.join(outdir, "molecules.rewrites"), "w") as fh:
        fh.write("\n".join(lines) + "\n")

    print(f"wrote {len(trees)} molecules, {len(reroots)} re-root rules, "
          f"{len(comms)} rotation + {len(dups)} duplicate + {len(terms)} terminal-multibond rules")


if __name__ == "__main__":
    main()
