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
