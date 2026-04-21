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

/// Shift free De Bruijn variables in a `RevExpr<ENodeOrVar<StitchLang>>` reachable from `root`.
///
/// `by` is the delta to apply to each free occurrence (so `by = 1` turns `$0` into `$1`).
/// `initial_depth` is the number of enclosing binders outside the root (usually 0 when the
/// root is a pattern top, or >0 when splicing under existing binders).
///
/// Meta-variable leaves (`ENodeOrVar::Var`) are untouched. A `lam` node increments the depth
/// for its single child. A `Var(n)` leaf is free iff `n >= depth`, in which case it's replaced
/// by `Var((n as i32 + by) as u32)`.
///
/// Visits each node at most once via a memo table, which assumes the DAG is well-formed
/// (every shared subterm seen at a consistent binder depth). Panics if a shift would produce
/// a negative index.
pub fn shift_free(expr: &mut RevExpr<egg::ENodeOrVar<crate::lang::StitchLang>>, root: egg::Id, by: i32, initial_depth: u32) {
    use rustc_hash::FxHashSet;
    let mut seen: FxHashSet<egg::Id> = FxHashSet::default();
    shift_free_rec(expr, root, by, initial_depth, &mut seen);
}

fn shift_free_rec(expr: &mut RevExpr<egg::ENodeOrVar<crate::lang::StitchLang>>, id: egg::Id, by: i32, depth: u32, seen: &mut rustc_hash::FxHashSet<egg::Id>) {
    use crate::lang::Op;
    if !seen.insert(id) {
        return;
    }
    // Handle leaf cases in one match; collect children for recursion if this is an interior node.
    let (child_depth, children): (u32, Vec<egg::Id>) = match &mut expr[id] {
        egg::ENodeOrVar::Var(_) => return,
        egg::ENodeOrVar::ENode(n) => match n.op {
            Op::Var(k) => {
                if k >= depth {
                    let shifted = k as i32 + by;
                    assert!(shifted >= 0, "shift_free produced a negative index");
                    n.op = Op::Var(shifted as u32);
                }
                return;
            }
            Op::Lam => (depth + 1, n.children.clone()),
            Op::Sym(_) => (depth, n.children.clone()),
        },
    };
    for c in children {
        shift_free_rec(expr, c, by, child_depth, seen);
    }
}

