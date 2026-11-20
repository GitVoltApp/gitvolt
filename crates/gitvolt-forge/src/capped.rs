//! A provider's session map with a ceiling (reactions, review threads, subscriptions, learned
//! avatars): a long session over many MRs/PRs and authors mustn't grow it without end. Past
//! `cap` entries, the least recently written tenth go (as `PipelineCache` drops its oldest).

use std::collections::HashMap;
use std::hash::Hash;

/// What the providers' maps keep at most.
pub const KEPT: usize = 2000;

pub struct Capped<K, V> {
    map: HashMap<K, (V, u64)>,
    /// The next write's stamp: higher is newer.
    next: u64,
    cap: usize,
}

impl<K: Eq + Hash, V> Default for Capped<K, V> {
    fn default() -> Self {
        Self::new(KEPT)
    }
}

impl<K: Eq + Hash, V> Capped<K, V> {
    pub fn new(cap: usize) -> Self {
        Self { map: HashMap::new(), next: 0, cap: cap.max(1) }
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k).map(|(v, _)| v)
    }

    pub fn contains_key(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Keeps `v` under `k` (the newest entry now); past the cap, the oldest tenth go.
    pub fn insert(&mut self, k: K, v: V) {
        self.map.insert(k, (v, self.next));
        self.next += 1;
        if self.map.len() > self.cap {
            let mut stamps: Vec<u64> = self.map.values().map(|(_, s)| *s).collect();
            stamps.sort_unstable();
            let cut = stamps[(self.cap / 10).max(self.map.len() - self.cap)];
            self.map.retain(|_, (_, s)| *s >= cut);
        }
    }

    /// Inserts `k` unless it's there: whether it was new (a set's `insert`).
    pub fn insert_new(&mut self, k: K, v: V) -> bool {
        if self.map.contains_key(&k) {
            return false;
        }
        self.insert(k, v);
        true
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        self.map.remove(k).map(|(v, _)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_the_cap_the_oldest_tenth_go() {
        let mut c = Capped::new(100);
        for i in 0..100 {
            c.insert(i, i);
        }
        assert_eq!(c.len(), 100);
        c.insert(0, 0);
        c.insert(100, 100);
        assert_eq!(c.len(), 91, "the ten oldest went at once, not one per insert");
        assert!(c.contains_key(&0), "written again: newest");
        assert!(!c.contains_key(&1) && !c.contains_key(&10) && c.contains_key(&11));
        assert!(!c.insert_new(100, 0) && c.insert_new(1, 1));
        assert_eq!(c.get(&100), Some(&100));
    }

    #[test]
    fn it_never_grows_past_the_cap() {
        let mut c = Capped::new(KEPT);
        for i in 0..KEPT * 5 {
            c.insert(i, ());
        }
        assert!(c.len() <= KEPT);
    }
}
