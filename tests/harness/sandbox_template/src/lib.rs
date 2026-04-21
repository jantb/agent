use std::collections::HashMap;
use std::hash::Hash;

/// Fixed-capacity cache that evicts the least-recently-used entry on overflow.
///
/// - `get` promotes the accessed key to most-recently-used.
/// - `put` inserts or updates; updating also promotes to MRU.
/// - When `len == capacity` and a new key is inserted, the LRU entry is evicted
///   and returned.
/// - A cache with `capacity == 0` never stores anything; `put` immediately
///   returns the provided pair as the "evicted" value.
pub struct LruCache<K, V>
where
    K: Eq + Hash + Clone,
{
    _k: std::marker::PhantomData<K>,
    _v: std::marker::PhantomData<V>,
}

impl<K, V> LruCache<K, V>
where
    K: Eq + Hash + Clone,
{
    pub fn new(_capacity: usize) -> Self {
        todo!("implement LruCache::new")
    }

    pub fn capacity(&self) -> usize {
        todo!("implement LruCache::capacity")
    }

    pub fn len(&self) -> usize {
        todo!("implement LruCache::len")
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the value for `key` and promotes it to most-recently-used.
    pub fn get(&mut self, _key: &K) -> Option<&V> {
        todo!("implement LruCache::get")
    }

    /// Inserts or updates `key`. On overflow (len == capacity before insert,
    /// and key was not already present) evicts the least-recently-used entry
    /// and returns `(evicted_key, evicted_value)`. Updating an existing key
    /// returns `None` and promotes it to MRU.
    pub fn put(&mut self, _key: K, _value: V) -> Option<(K, V)> {
        todo!("implement LruCache::put")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_is_empty() {
        let c: LruCache<&str, i32> = LruCache::new(4);
        assert_eq!(c.len(), 0);
        assert_eq!(c.capacity(), 4);
        assert!(c.is_empty());
    }

    #[test]
    fn put_and_get_roundtrip() {
        let mut c = LruCache::new(2);
        assert_eq!(c.put("a", 1), None);
        assert_eq!(c.put("b", 2), None);
        assert_eq!(c.get(&"a"), Some(&1));
        assert_eq!(c.get(&"b"), Some(&2));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn get_missing_returns_none() {
        let mut c: LruCache<&str, i32> = LruCache::new(2);
        assert_eq!(c.get(&"missing"), None);
    }

    #[test]
    fn overflow_evicts_lru() {
        let mut c = LruCache::new(2);
        c.put("a", 1);
        c.put("b", 2);
        // a is LRU; inserting c evicts a.
        assert_eq!(c.put("c", 3), Some(("a", 1)));
        assert_eq!(c.get(&"a"), None);
        assert_eq!(c.get(&"b"), Some(&2));
        assert_eq!(c.get(&"c"), Some(&3));
    }

    #[test]
    fn get_promotes_to_mru() {
        let mut c = LruCache::new(2);
        c.put("a", 1);
        c.put("b", 2);
        // Touch a → b becomes LRU.
        assert_eq!(c.get(&"a"), Some(&1));
        assert_eq!(c.put("c", 3), Some(("b", 2)));
        assert_eq!(c.get(&"a"), Some(&1));
        assert_eq!(c.get(&"b"), None);
    }

    #[test]
    fn put_existing_key_updates_and_promotes() {
        let mut c = LruCache::new(2);
        c.put("a", 1);
        c.put("b", 2);
        // Update a → a is MRU, b is LRU.
        assert_eq!(c.put("a", 10), None);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&"a"), Some(&10));
        // Insert c → evicts b, not a.
        assert_eq!(c.put("c", 3), Some(("b", 2)));
    }

    #[test]
    fn zero_capacity_stores_nothing() {
        let mut c: LruCache<&str, i32> = LruCache::new(0);
        assert_eq!(c.put("a", 1), Some(("a", 1)));
        assert_eq!(c.len(), 0);
        assert_eq!(c.get(&"a"), None);
    }

    #[test]
    fn works_with_owned_string_keys() {
        let mut c: LruCache<String, Vec<u8>> = LruCache::new(2);
        c.put("x".to_string(), vec![1, 2, 3]);
        c.put("y".to_string(), vec![4]);
        assert_eq!(c.get(&"x".to_string()), Some(&vec![1, 2, 3]));
        c.put("z".to_string(), vec![]);
        assert_eq!(c.get(&"y".to_string()), None);
    }
}

// Silence unused-import warning on the stubbed impl.
#[allow(dead_code)]
fn _unused(_: HashMap<u8, u8>) {}
