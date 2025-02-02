use std::collections::{HashMap, HashSet};

use hashlink::LinkedHashMap;
use pool::traits::Recyclable;

/// guarantees that the underlaying elements are copyable,
/// besides that it's simply a `clone_from`
pub trait CloneWithCopyableElements {
    fn copy_clone_from(&mut self, other: &Self);
}

impl<T> CloneWithCopyableElements for Vec<T>
where
    T: Copy + Clone,
{
    fn copy_clone_from(&mut self, other: &Self) {
        self.clone_from(other);
    }
}

impl<K, V> CloneWithCopyableElements for HashMap<K, V>
where
    K: Copy + Clone,
    V: Copy + Clone,
{
    fn copy_clone_from(&mut self, other: &Self) {
        self.clone_from(other);
    }
}

impl<K, V> CloneWithCopyableElements for LinkedHashMap<K, V>
where
    K: Copy + Clone + std::cmp::Eq + std::hash::Hash,
    V: Copy + Clone,
{
    fn copy_clone_from(&mut self, other: &Self) {
        self.clone_from(other);
    }
}

impl<K> CloneWithCopyableElements for HashSet<K>
where
    K: Copy + Clone,
{
    fn copy_clone_from(&mut self, other: &Self) {
        self.clone_from(other);
    }
}

pub trait ReusableCore: CloneWithCopyableElements + Recyclable {}
