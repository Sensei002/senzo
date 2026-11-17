use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::OnceCell;

use crate::MetadataError;

type CachedBytes = Arc<[u8]>;
type InFlight = Arc<OnceCell<Result<CachedBytes, MetadataError>>>;

struct CacheEntry {
    body: CachedBytes,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
    in_flight: HashMap<String, InFlight>,
    hits: u64,
    misses: u64,
}

pub(crate) struct MetadataCache {
    capacity: usize,
    state: Mutex<CacheState>,
}

impl MetadataCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(CacheState::default()),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<CachedBytes>, MetadataError> {
        let mut state = self.state.lock().map_err(|_| MetadataError::CachePoisoned)?;
        let expired = state
            .entries
            .get(key)
            .is_some_and(|entry| entry.expires_at <= Instant::now());
        if expired {
            state.entries.remove(key);
            state.order.retain(|candidate| candidate != key);
        }

        if let Some(entry) = state.entries.get(key) {
            let body = Arc::clone(&entry.body);
            state.order.retain(|candidate| candidate != key);
            state.order.push_back(key.to_owned());
            state.hits += 1;
            return Ok(Some(body));
        }

        state.misses += 1;
        Ok(None)
    }

    pub fn insert(
        &self,
        key: String,
        body: CachedBytes,
        ttl: Duration,
    ) -> Result<(), MetadataError> {
        if ttl.is_zero() {
            return Ok(());
        }
        let mut state = self.state.lock().map_err(|_| MetadataError::CachePoisoned)?;
        state.entries.insert(
            key.clone(),
            CacheEntry {
                body,
                expires_at: Instant::now() + ttl,
            },
        );
        state.order.retain(|candidate| candidate != &key);
        state.order.push_back(key);

        while state.entries.len() > self.capacity {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.entries.remove(&oldest);
        }
        Ok(())
    }

    pub fn in_flight(&self, key: &str) -> Result<InFlight, MetadataError> {
        let mut state = self.state.lock().map_err(|_| MetadataError::CachePoisoned)?;
        Ok(Arc::clone(
            state
                .in_flight
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(OnceCell::new())),
        ))
    }

    pub fn finish_in_flight(&self, key: &str, cell: &InFlight) -> Result<(), MetadataError> {
        let mut state = self.state.lock().map_err(|_| MetadataError::CachePoisoned)?;
        if state
            .in_flight
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, cell))
        {
            state.in_flight.remove(key);
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<CacheStats, MetadataError> {
        let state = self.state.lock().map_err(|_| MetadataError::CachePoisoned)?;
        Ok(CacheStats {
            hits: state.hits,
            misses: state.misses,
            entries: state.entries.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_evicts_the_least_recently_used_entry_and_expires() {
        let cache = MetadataCache::new(2);
        cache
            .insert("a".into(), Arc::from(&b"one"[..]), Duration::from_secs(10))
            .unwrap();
        cache
            .insert("b".into(), Arc::from(&b"two"[..]), Duration::from_secs(10))
            .unwrap();
        assert_eq!(&*cache.get("a").unwrap().unwrap(), b"one");
        cache
            .insert("c".into(), Arc::from(&b"three"[..]), Duration::from_secs(10))
            .unwrap();
        assert!(cache.get("b").unwrap().is_none());
        assert_eq!(&*cache.get("a").unwrap().unwrap(), b"one");
        assert_eq!(cache.stats().unwrap().entries, 2);
    }

    #[test]
    fn zero_ttl_does_not_store_a_response() {
        let cache = MetadataCache::new(2);
        cache
            .insert("a".into(), Arc::from(&b"one"[..]), Duration::ZERO)
            .unwrap();
        assert!(cache.get("a").unwrap().is_none());
    }
}
