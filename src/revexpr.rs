/// Like an egg::RecExpr but with the nodes in reverse order and publicly accessible
/// This is much better for representing partial patterns as expanding can just
/// append to the end of the vector, and also doesn't need to worry about shifting child Ids
/// for nodes within the vector.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RevExpr<L: egg::Language> {
    pub nodes: Vec<L>,
}

impl<L: egg::Language> RevExpr<L> {
    /// Creates a `RevExpr` from a pre-built node list (root at index 0).
    pub fn new(nodes: Vec<L>) -> Self {
        Self { nodes }
    }
}

/// Reverses the nodes in the vector of nodes and updates the children ids to point to the correct nodes
fn rev_nodes<L: egg::Language>(nodes: &mut Vec<L>) {
    nodes.reverse();
    let max_id = nodes.len() - 1;
    for node in nodes {
        for child in node.children_mut() {
            *child = egg::Id::from(max_id - usize::from(*child));
        }
    }
}

impl<L: egg::FromOp> std::str::FromStr for RevExpr<L> {
    type Err = egg::RecExprParseError<L::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let recexpr = s.parse::<egg::RecExpr<L>>()?;
        Ok(recexpr.into())
    }
}

impl<L: egg::Language + std::fmt::Display> std::fmt::Display for RevExpr<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // somewhat silly clone now but it's okay – display isn't performance critical and isn't a huge clone
        let recexpr: egg::RecExpr<L> = self.clone().into();
        std::fmt::Display::fmt(&recexpr, f)
    }
}

impl<L: egg::Language> std::ops::Index<egg::Id> for RevExpr<L> {
    type Output = L;
    fn index(&self, index: egg::Id) -> &Self::Output {
        &self.nodes[usize::from(index)]
    }
}

impl<L: egg::Language> std::ops::IndexMut<egg::Id> for RevExpr<L> {
    fn index_mut(&mut self, index: egg::Id) -> &mut Self::Output {
        &mut self.nodes[usize::from(index)]
    }
}

impl<L: egg::Language> From<RevExpr<L>> for egg::RecExpr<L> {
    fn from(rev_expr: RevExpr<L>) -> Self {
        let mut nodes: Vec<L> = rev_expr.nodes;
        rev_nodes(&mut nodes);
        egg::RecExpr::from(nodes)
    }
}

impl<L: egg::Language> From<egg::RecExpr<L>> for RevExpr<L> {
    fn from(recexpr: egg::RecExpr<L>) -> Self {
        let mut nodes: Vec<L> = recexpr.into();
        rev_nodes(&mut nodes);
        RevExpr::new(nodes)
    }
}

/// Wrap `expr` rooted at `root` so the result is a closed function of `d_k`
/// pattern-internal binders followed by `hoist.len()` outer hoist binders.
///
/// `hoist` is in the **post-pattern-wrap frame** (sorted ascending): each
/// `hoist[p]` is one of the indices that remains free after the inner `d_k`
/// pattern-lams would have absorbed indices `0..d_k-1`. Equivalently, an
/// original-frame free index `i >= d_k` maps to `hoist[?] == i - d_k`.
///
/// Index rewriting per leaf at body-depth `d` with original index `i`:
/// - `i < d`: bound by an internal lam in the body — left alone.
/// - `i >= d`, with effective `e = i - d`:
///   * `e < d_k`: bound by the `e`-th inner pattern-lam (counting from inner=0).
///   * `e >= d_k`, `h = e - d_k` in `hoist` at position `p`: bound by the
///     `p`-th hoist-lam from outer (the outermost binds `hoist[0]`, the
///     smallest); at body-frame depth `n + d_k - 1 - p` from inside, so the
///     leaf's new index is `d + (n + d_k - 1 - p)`.
///   * `e >= d_k` and `h` not in `hoist`: panics. Callers must ensure every
///     post-pattern-wrap free index is in `hoist` (else the result is open).
///
/// The output is wrapped in `d_k + n` `lam` nodes via `L::from_op("lam", …)`.
/// Memoised on `(Id, depth)`: the same shared subterm may be visited at
/// different body depths and need different rewritten leaves each time.
pub fn abstract_with_hoist<L>(expr: &egg::RecExpr<L>, root: egg::Id, d_k: u32, hoist: &[u32]) -> egg::RecExpr<L>
where
    L: egg::Language + egg::FromOp,
    L::Error: std::fmt::Debug,
    L::Discriminant: crate::lang::StitchDisc,
{
    let n = hoist.len() as u32;
    let mut out = egg::RecExpr::default();
    let mut memo: rustc_hash::FxHashMap<(egg::Id, u32), egg::Id> = rustc_hash::FxHashMap::default();
    let body_id = abstract_walk(expr, root, 0, d_k, n, hoist, &mut out, &mut memo);
    let mut id = body_id;
    for _ in 0..(d_k + n) {
        let lam_node = L::from_op("lam", vec![id]).expect("from_op lam");
        id = out.add(lam_node);
    }
    let _ = id;
    out
}

#[allow(clippy::too_many_arguments)]
fn abstract_walk<L>(expr: &egg::RecExpr<L>, id: egg::Id, depth: u32, d_k: u32, n: u32, hoist: &[u32], out: &mut egg::RecExpr<L>, memo: &mut rustc_hash::FxHashMap<(egg::Id, u32), egg::Id>) -> egg::Id
where
    L: egg::Language + egg::FromOp,
    L::Error: std::fmt::Debug,
    L::Discriminant: crate::lang::StitchDisc,
{
    use crate::lang::StitchDisc;
    if let Some(&hit) = memo.get(&(id, depth)) {
        return hit;
    }
    let node = &expr[id];
    let disc = node.discriminant();
    let new_id = if let Some(i) = disc.de_bruijn_index() {
        let new_i = if i < depth {
            i
        } else {
            let e = i - depth;
            if e < d_k {
                e + depth
            } else {
                let h_post = e - d_k;
                match hoist.iter().position(|&x| x == h_post) {
                    Some(p) => depth + (n + d_k - 1 - p as u32),
                    None => panic!("abstract_with_hoist: free index ${e} not in hoist set {hoist:?}"),
                }
            }
        };
        let leaf = L::from_op(&format!("${new_i}"), vec![]).expect("from_op DB var");
        out.add(leaf)
    } else {
        let new_kids: Vec<egg::Id> = node
            .children()
            .iter()
            .enumerate()
            .map(|(j, &c)| {
                let child_depth = depth + if disc.binds_child(j) { 1 } else { 0 };
                abstract_walk(expr, c, child_depth, d_k, n, hoist, out, memo)
            })
            .collect();
        let mut new_node = node.clone();
        for (slot, kid) in new_node.children_mut().iter_mut().zip(new_kids.iter()) {
            *slot = *kid;
        }
        out.add(new_node)
    };
    memo.insert((id, depth), new_id);
    new_id
}

/// Shift every *free* De Bruijn index reachable from `root`. `by` can be positive or negative (but must not underflow to negative).
/// The idea here is we can use this to take an expression like (+ $6 $7) and know that we're hoisting it through 5 levels of lambda
/// so it should now be (+ $1 $2).
pub fn shift_free<L>(expr: &mut RevExpr<L>, root: egg::Id, by: i32, initial_depth: u32)
where
    L: egg::Language + egg::FromOp,
    L::Error: std::fmt::Debug,
    L::Discriminant: crate::lang::StitchDisc,
{
    let mut seen: rustc_hash::FxHashSet<egg::Id> = rustc_hash::FxHashSet::default();
    shift_free_rec(expr, root, by, initial_depth, &mut seen);
}

fn shift_free_rec<L>(expr: &mut RevExpr<L>, id: egg::Id, by: i32, depth: u32, seen: &mut rustc_hash::FxHashSet<egg::Id>)
where
    L: egg::Language + egg::FromOp,
    L::Error: std::fmt::Debug,
    L::Discriminant: crate::lang::StitchDisc,
{
    use crate::lang::StitchDisc;
    if !seen.insert(id) {
        return;
    }
    let disc = expr[id].discriminant();
    if let Some(n) = disc.de_bruijn_index() {
        if n >= depth {
            let shifted = n as i32 + by;
            assert!(shifted >= 0, "shift_free: negative index after shifting ${n} by {by}");
            let new_node = L::from_op(&format!("${shifted}"), vec![]).expect("from_op for shifted DB var");
            expr[id] = new_node;
        }
        return;
    }
    // clone to mutate as we go.
    let kids: Vec<(usize, egg::Id)> = expr[id].children().iter().copied().enumerate().collect();
    for (j, child) in kids {
        let child_depth = depth + if disc.binds_child(j) { 1 } else { 0 };
        shift_free_rec(expr, child, by, child_depth, seen);
    }
}
