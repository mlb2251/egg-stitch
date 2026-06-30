//! The stateful footprint dedup tracker (see the [module docs](super)).
//! [`FootprintTracker`] mirrors [`crate::search::SeenTracker`] but keys on the
//! permutation-invariant signature; a cheap [`cheap_proxy_of`] proxy gates the
//! expensive [`compute`].

use super::equivalence::footprint_equivalent;
use super::id_u32;
use super::signature::{FootprintScratch, compute};
use crate::hashing::{h64, mix64};
use crate::lang::{LanguageFamily, StitchOp};
use crate::matching::MatchAtEClass;
use crate::search::SearchState;
use rustc_hash::FxHashMap;
use std::time::{Duration, Instant};

#[cfg(test)]
use super::signature::{Footprint, footprint_of};

/// Salt for the cheap proxy hash, domain-separated from the signature's salts so
/// a proxy can't alias a column/marginal/factor/root hash.
const SALT_PROXY: u64 = 0x9001_9001;

/// A cheap, permutation-invariant *proxy* for a candidate's footprint: a hash of
/// `(arity, num_matches, ⊕ over matches of mix(root | num_substs<<32))`. It
/// depends only on quantities the full signature also quotients invariant (root
/// ids, per-root subst counts, arity, match count) and never on variable identity
/// or order, so **footprint-equal ⇒ equal proxy**. One `mix64` per match, no
/// per-factor/row work — far cheaper than [`compute`].
fn cheap_proxy_of(matches: &[MatchAtEClass], arity: usize) -> u64 {
    let mut acc: u64 = 0;
    for m in matches {
        // Commutative combine (wrapping add) keeps it order-invariant.
        acc = acc.wrapping_add(mix64(id_u32(m.root_eclass) as u64 | ((m.num_substs() as u64) << 32)));
    }
    h64(SALT_PROXY, &(arity, matches.len(), acc))
}

/// Tracks already-seen match footprints to dedupe successors, mirroring
/// [`crate::search::SeenTracker`] but keyed on the permutation-invariant
/// signature instead of pattern structure.
///
/// Two-level: a cheap invariant [`cheap_proxy_of`] gates the expensive 128-bit
/// [`compute`]. The first candidate to claim a proxy is kept without computing
/// its signature (a fresh proxy *guarantees* it differs from everything seen) —
/// but it is *not* discarded: we retain a [`RepHandle`] (the candidate's node
/// index) so its match set, which already lives for the whole search in the node
/// array, can be re-read on demand. On the first proxy collision we materialise
/// that deferred representative — computing its signature lazily, only now that a
/// peer actually needs comparing against it — and then dedup the newcomer.
///
/// Each bucket entry is `(sig, frozen marginals, min size)`; the frozen marginals are
/// the most-flexible set seen for that sig (a *marginal multiset*, compared up to
/// the variable permutation the sig quotients out), and a repeat is a hit when a
/// recorded entry's frozen marginals are a sub-multiset of the new ones — it was at
/// least as flexible, so this visit's successors are already reachable.
///
/// Cost: signatures are still computed only on genuine proxy collisions (the
/// common unique-proxy candidate pays only the proxy hash), so the fast path is
/// preserved; the lazy materialisation adds exactly one extra `compute` per
/// colliding bucket (the representative's), and no match set is ever cloned —
/// only its node index is stored. Unlike the older scheme, the representative's
/// own duplicates are now caught, so dedup is exact up to footprint-equality.
#[derive(Default)]
pub struct FootprintTracker {
    /// Per proxy: the bucket's deferred representative plus its materialised entries.
    buckets: FxHashMap<u64, Bucket>,
    /// Signature-computation buffers, reused across candidates so the hot path allocates nothing.
    scratch: FootprintScratch,
    pub hits: usize,
    /// Candidates kept via the unique-proxy fast path (full signatures avoided).
    pub proxy_skips: usize,
    pub capped: usize,
    pub time: Duration,
}

/// One proxy's bucket: a not-yet-materialised representative (kept as a lazy
/// handle until a collision forces its signature) and the signatures recorded so
/// far. `rep` is `Some` only between the fresh-proxy claim and the first
/// collision; after that all members live in `entries`.
#[derive(Default)]
struct Bucket {
    rep: Option<RepHandle>,
    /// Each entry: `(signature, frozen marginals, min size, node id)`. The node id
    /// resolves the entry's match set on demand for the `check_slow` validation.
    entries: Vec<(u128, Vec<u64>, usize, usize)>,
}

/// A deferred representative: enough to materialise its bucket entry on demand
/// without retaining its match set. The match set is re-read from the search's
/// (append-only) node array via `id`; `frozen` (whose length is the arity) and
/// `size` are the small per-candidate facts the signature alone doesn't carry.
struct RepHandle {
    id: usize,
    frozen: Vec<bool>,
    size: usize,
}

impl FootprintTracker {
    pub fn new() -> Self {
        Self::default()
    }
    /// Number of distinct proxy buckets recorded.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Dedupes `state` at `frozen`, with `size = compute_pattern_size(pattern)`.
    /// `id` is the node index this candidate will occupy (so a deferred
    /// representative can re-read its match set later), and `resolve` maps a node
    /// index back to its stored match set. Returns `true` (skip) on a dominated
    /// repeat.
    pub fn check_state<'n, F: LanguageFamily, O: StitchOp>(&mut self, state: &SearchState<F, O>, frozen: &[bool], size: usize, id: usize, check_slow: bool, resolve: &impl Fn(usize) -> &'n [MatchAtEClass]) -> bool {
        let t = Instant::now();
        let skip = self.check_core(&state.matches, state.pattern.vars.len(), frozen, size, id, check_slow, resolve);
        self.time += t.elapsed();
        skip
    }

    /// Core dedup over raw match data (split out from [`check_state`] so it can be
    /// unit-tested with a plain match-set resolver). See the type docs for the
    /// lazy-representative scheme. With `check_slow`, every signature equality is
    /// validated against the exact [`footprint_equivalent`] relation before being
    /// trusted — catching a hash collision or over-merge rather than silently
    /// pruning a genuinely distinct candidate.
    #[allow(clippy::too_many_arguments)]
    fn check_core<'n>(&mut self, matches: &[MatchAtEClass], arity: usize, frozen: &[bool], size: usize, id: usize, check_slow: bool, resolve: &impl Fn(usize) -> &'n [MatchAtEClass]) -> bool {
        let proxy = cheap_proxy_of(matches, arity);
        let rep = {
            let bucket = self.buckets.entry(proxy).or_default();
            // Fresh proxy: defer the signature, keeping only a handle to the
            // candidate so its duplicates can still be caught later.
            if bucket.rep.is_none() && bucket.entries.is_empty() {
                bucket.rep = Some(RepHandle { id, frozen: frozen.to_vec(), size });
                self.proxy_skips += 1;
                return false;
            }
            bucket.rep.take()
        };
        // First collision in this bucket: materialise the deferred representative
        // from its retained match set before comparing the newcomer against it.
        if let Some(rep) = rep {
            let (rsig, rmarginals, rcapped) = compute(resolve(rep.id), rep.frozen.len(), &mut self.scratch);
            if rcapped {
                self.capped += 1;
            }
            let rfc = frozen_marginals(&rep.frozen, &rmarginals);
            self.buckets.get_mut(&proxy).expect("present").entries.push((rsig, rfc, rep.size, rep.id));
        }
        let (sig, marginals, capped) = compute(matches, arity, &mut self.scratch);
        if capped {
            self.capped += 1;
        }
        // Slow guard: a signature equality must reflect a genuine footprint
        // equivalence. Re-read each same-sig entry's match set and confirm an
        // exact column-permutation witness before any dedup decision rests on it.
        if check_slow {
            for e in &self.buckets.get(&proxy).expect("present").entries {
                if e.0 == sig {
                    assert!(footprint_equivalent(resolve(e.3), matches, arity), "footprint signature collision: distinct match sets share signature {sig:#034x}");
                }
            }
        }
        let fc = frozen_marginals(frozen, &marginals);
        let skip = dedup_in_bucket(&mut self.buckets.get_mut(&proxy).expect("present").entries, sig, fc, size, id);
        if skip {
            self.hits += 1;
        }
        skip
    }

    /// Records a precomputed footprint at pattern `size` (used by tests; always
    /// exercises the signature path via a single shared bucket).
    #[cfg(test)]
    fn check_and_insert(&mut self, fp: &Footprint, frozen: &[bool], size: usize) -> bool {
        if fp.capped {
            self.capped += 1;
        }
        let fc = frozen_marginals(frozen, &fp.marginals);
        let skip = dedup_in_bucket(&mut self.buckets.entry(0).or_default().entries, fp.sig, fc, size, 0);
        if skip {
            self.hits += 1;
        }
        skip
    }
}

/// The frozen variables' marginals, sorted — a marginal multiset compared up to the
/// variable permutation the signature quotients out.
fn frozen_marginals(frozen: &[bool], marginals: &[u64]) -> Vec<u64> {
    let mut fc: Vec<u64> = frozen.iter().enumerate().filter(|(_, b)| **b).map(|(v, _)| marginals[v]).collect();
    fc.sort_unstable();
    fc
}

/// Subsumption within one proxy bucket. Skip only if a recorded same-`sig` entry
/// dominates the newcomer on *both* axes: its frozen marginals are a sub-multiset
/// of `fc` (at least as flexible — successors already reachable) **and** its
/// pattern size is `≤ size` (at least as cheap — `best` not worse). Otherwise
/// keep, and fold the newcomer's (more-flexible) frozen set and (smaller) size
/// into the entry. Footprint-equal patterns differ only in definition size, so
/// the size guard ensures a strictly smaller equivalent is never pruned away.
fn dedup_in_bucket(bucket: &mut Vec<(u128, Vec<u64>, usize, usize)>, sig: u128, fc: Vec<u64>, size: usize, id: usize) -> bool {
    match bucket.iter_mut().find(|(s, ..)| *s == sig) {
        Some(entry) if submultiset(&entry.1, &fc) && entry.2 <= size => true,
        Some(entry) => {
            entry.1 = fc;
            entry.2 = entry.2.min(size);
            false
        }
        None => {
            bucket.push((sig, fc, size, id));
            false
        }
    }
}

/// True iff sorted `a` is a sub-multiset of sorted `b` (the marginal-space analogue
/// of `frozen_subset`: a smaller frozen set is at least as flexible).
fn submultiset(a: &[u64], b: &[u64]) -> bool {
    let mut j = 0;
    for &x in a {
        while j < b.len() && b[j] < x {
            j += 1;
        }
        if j == b.len() || b[j] != x {
            return false;
        }
        j += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::test_helpers::match_at;

    /// Frozen-marginal subsumption: freezing the *other* of two swapped variables
    /// is the same frozen set up to the permutation, so the second visit is a hit.
    #[test]
    fn frozen_subsumption_up_to_permutation() {
        let mut t = FootprintTracker::new();
        let p1 = footprint_of(&[match_at(100, 2, vec![vec![10, 11]])], 2);
        let p2 = footprint_of(&[match_at(100, 2, vec![vec![11, 10]])], 2);
        assert!(!t.check_and_insert(&p1, &[true, false], 5));
        assert!(t.check_and_insert(&p2, &[false, true], 5));
    }

    /// Lazy representative: the *first* duplicate of a bucket's representative is
    /// caught. The representative claims its proxy as a deferred handle (no sig);
    /// when its footprint-equal peer arrives the representative is materialised
    /// from its retained match set and the peer is pruned. Resolver maps node ids
    /// back to their stored match sets, as the search's node array does.
    #[test]
    fn lazy_representative_catches_first_duplicate() {
        let mut t = FootprintTracker::new();
        let store = [vec![match_at(100, 2, vec![vec![10, 11]])], vec![match_at(100, 2, vec![vec![10, 11]])]];
        let resolve = |i: usize| &store[i][..];
        // Representative kept (deferred), then its identical-footprint peer pruned.
        assert!(!t.check_core(&store[0], 2, &[false, false], 5, 0, true, &resolve));
        assert!(t.check_core(&store[1], 2, &[false, false], 5, 1, true, &resolve));
        assert_eq!(t.proxy_skips, 1);
        assert_eq!(t.hits, 1);
    }

    /// A commutative swap (footprint-equal, rows column-permuted) is also caught
    /// as the representative's first duplicate, not just a literal repeat.
    #[test]
    fn lazy_representative_catches_permuted_duplicate() {
        let mut t = FootprintTracker::new();
        let store = [vec![match_at(100, 2, vec![vec![10, 11]])], vec![match_at(100, 2, vec![vec![11, 10]])]];
        let resolve = |i: usize| &store[i][..];
        assert!(!t.check_core(&store[0], 2, &[false, false], 5, 0, true, &resolve));
        assert!(t.check_core(&store[1], 2, &[false, false], 5, 1, true, &resolve));
    }

    /// Cost guard: a footprint-equal pattern that is *strictly smaller* is never
    /// pruned (it could become a cheaper `best`); a same-or-bigger one still is.
    #[test]
    fn smaller_equivalent_not_pruned() {
        let mut t = FootprintTracker::new();
        let big = footprint_of(&[match_at(100, 1, vec![vec![10]])], 1);
        // Record the big one first (size 5).
        assert!(!t.check_and_insert(&big, &[false], 5));
        // A footprint-equal but smaller (size 3) pattern must survive.
        assert!(!t.check_and_insert(&big, &[false], 3));
        // A footprint-equal bigger (size 7) one is now dominated → pruned.
        assert!(t.check_and_insert(&big, &[false], 7));
    }
}
