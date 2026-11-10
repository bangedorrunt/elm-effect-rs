use std::collections::BTreeSet;

/// The set of states active at one instant.
///
/// A configuration holds leaf states. A parallel state contributes one leaf
/// per region, so its configuration has several leaves. The set is ordered,
/// which makes the primary leaf, entry order, and every trace deterministic.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(bound(
    serialize = "S: serde::Serialize + Ord",
    deserialize = "S: serde::Deserialize<'de> + Ord"
))]
pub struct Configuration<S> {
    leaves: BTreeSet<S>,
}

impl<S: Ord + Clone> Configuration<S> {
    /// An empty configuration, before the machine is entered.
    #[must_use]
    pub fn new() -> Self {
        Self {
            leaves: BTreeSet::new(),
        }
    }

    /// A configuration with exactly one leaf.
    #[must_use]
    pub fn single(state: S) -> Self {
        let mut leaves = BTreeSet::new();
        leaves.insert(state);
        Self { leaves }
    }

    /// A configuration from any list of leaves. Duplicates collapse.
    #[must_use]
    pub fn from_leaves(leaves: impl IntoIterator<Item = S>) -> Self {
        Self {
            leaves: leaves.into_iter().collect(),
        }
    }

    /// Add a leaf. Returns `false` when it was already present.
    pub fn insert(&mut self, state: S) -> bool {
        self.leaves.insert(state)
    }

    /// Remove a leaf. Returns `false` when it was absent.
    pub fn remove(&mut self, state: &S) -> bool {
        self.leaves.remove(state)
    }

    /// Whether this leaf is active.
    #[must_use]
    pub fn contains(&self, state: &S) -> bool {
        self.leaves.contains(state)
    }

    /// The active leaves, in ascending order.
    pub fn leaves(&self) -> impl Iterator<Item = &S> {
        self.leaves.iter()
    }

    /// The first active leaf, which is the stable answer for a machine with
    /// one active region.
    #[must_use]
    pub fn primary(&self) -> Option<&S> {
        self.leaves.iter().next()
    }

    /// How many leaves are active.
    #[must_use]
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Whether no leaf is active.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }
}

impl<S: Ord + Clone> From<S> for Configuration<S> {
    fn from(state: S) -> Self {
        Self::single(state)
    }
}

impl<S: Ord + Clone> From<&S> for Configuration<S> {
    fn from(state: &S) -> Self {
        Self::single(state.clone())
    }
}

impl<S: Ord + Clone> FromIterator<S> for Configuration<S> {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        Self::from_leaves(iter)
    }
}

impl<'a, S: Ord + Clone> IntoIterator for &'a Configuration<S> {
    type Item = &'a S;
    type IntoIter = std::collections::btree_set::Iter<'a, S>;

    fn into_iter(self) -> Self::IntoIter {
        self.leaves.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_are_ordered_and_deduplicated() {
        let config = Configuration::from_leaves(["b", "a", "b"]);
        assert_eq!(config.leaves().copied().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(config.primary(), Some(&"a"));
        assert_eq!(config.len(), 2);
        assert!(!config.is_empty());
    }

    #[test]
    fn from_a_reference_clones_one_leaf() {
        let config: Configuration<String> = (&"x".to_string()).into();
        assert_eq!(config.leaves().cloned().collect::<Vec<_>>(), ["x"]);
    }
}
