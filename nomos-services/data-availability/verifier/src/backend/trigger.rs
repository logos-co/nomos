use std::{
    collections::HashMap,
    hash::Hash,
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

pub struct MempoolPublishTriggerConfig {
    pub publish_threshold: f64,
    pub share_timeout: Duration,
    pub prune_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareState {
    Complete,
    Incomplete,
    Expired,
}

struct ShareEntry {
    count: AtomicU16,
    created_at: Instant,
    assignations: u16,
    expired: AtomicBool,
}

pub struct MempoolPublishTrigger<Id> {
    config: MempoolPublishTriggerConfig,
    received: Arc<RwLock<HashMap<Id, Arc<ShareEntry>>>>,
}

impl<Id: Clone + Hash + Eq> MempoolPublishTrigger<Id> {
    pub fn new(config: MempoolPublishTriggerConfig) -> Self {
        Self {
            config,
            received: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn update(&self, blob_id: Id, assignations: u16) -> ShareState {
        let maybe_entry = self.received.read().unwrap().get(&blob_id).cloned();

        let entry = match maybe_entry {
            Some(entry) => entry,
            None => {
                let mut map = self.received.write().unwrap();
                map.entry(blob_id)
                    .or_insert_with(|| {
                        Arc::new(ShareEntry {
                            count: AtomicU16::new(0),
                            created_at: Instant::now(),
                            assignations,
                            expired: AtomicBool::new(false),
                        })
                    })
                    .clone()
            }
        };

        if entry.expired.load(Ordering::Acquire) {
            return ShareState::Expired;
        }

        let new_count = entry.count.fetch_add(1, Ordering::AcqRel) + 1;

        if new_count >= entry.assignations {
            ShareState::Complete
        } else {
            ShareState::Incomplete
        }
    }

    pub fn prune(&self) -> Vec<Id> {
        let now = Instant::now();
        let mut to_publish = Vec::new();

        let mut map = self.received.write().unwrap();

        map.retain(|blob_id, state| {
            let elapsed = now.duration_since(state.created_at);

            if elapsed >= self.config.prune_timeout {
                return false;
            }

            if !state.expired.load(Ordering::Acquire) && elapsed >= self.config.share_timeout {
                state.expired.store(true, Ordering::Release);

                let count = state.count.load(Ordering::Acquire);
                let threshold =
                    (state.assignations as f64 * self.config.publish_threshold).ceil() as u16;

                if count >= threshold {
                    to_publish.push(blob_id.clone());
                }
            }

            true
        });

        to_publish
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    const BLOB_ID: [u8; 32] = [1; 32];

    impl<Id: Hash + Eq> MempoolPublishTrigger<Id> {
        fn len(&self) -> usize {
            self.received.read().unwrap().len()
        }

        fn get_count(&self, blob_id: &Id) -> Option<u16> {
            self.received
                .read()
                .unwrap()
                .get(blob_id)
                .map(|e| e.count.load(Ordering::Relaxed))
        }
    }

    fn test_config() -> MempoolPublishTriggerConfig {
        MempoolPublishTriggerConfig {
            publish_threshold: 0.5,
            share_timeout: Duration::from_millis(50),
            prune_timeout: Duration::from_millis(100),
        }
    }

    #[test]
    fn test_update_reaches_complete() {
        let trigger = MempoolPublishTrigger::new(test_config());
        let assignations = 3;

        assert_eq!(
            trigger.update(BLOB_ID, assignations),
            ShareState::Incomplete
        );
        assert_eq!(trigger.get_count(&BLOB_ID), Some(1));

        assert_eq!(
            trigger.update(BLOB_ID, assignations),
            ShareState::Incomplete
        );
        assert_eq!(trigger.get_count(&BLOB_ID), Some(2));

        assert_eq!(trigger.update(BLOB_ID, assignations), ShareState::Complete);
        assert_eq!(trigger.get_count(&BLOB_ID), Some(3));

        // Additional updates should still report Complete
        assert_eq!(trigger.update(BLOB_ID, assignations), ShareState::Complete);
        assert_eq!(trigger.get_count(&BLOB_ID), Some(4));
    }

    #[test]
    fn test_prune_and_publish() {
        let trigger = MempoolPublishTrigger::new(test_config());

        // 6 out of 10 shares received (60%).
        for _ in 0..6 {
            trigger.update(BLOB_ID, 10);
        }

        thread::sleep(test_config().share_timeout);

        let to_publish = trigger.prune();
        assert_eq!(to_publish, vec![BLOB_ID]);
        assert_eq!(trigger.len(), 1);
    }

    #[test]
    fn test_prune_and_not_publish() {
        let trigger = MempoolPublishTrigger::new(test_config());

        // 4 out of 10 shares received (40%), which is < 50% threshold.
        for _ in 0..4 {
            trigger.update(BLOB_ID, 10);
        }

        thread::sleep(test_config().share_timeout);

        let to_publish = trigger.prune();
        assert!(to_publish.is_empty());
        assert_eq!(trigger.len(), 1);
    }

    #[test]
    fn test_prune_removes_old_entry() {
        let trigger = MempoolPublishTrigger::new(test_config());

        trigger.update(BLOB_ID, 10);
        assert_eq!(trigger.len(), 1);

        thread::sleep(test_config().prune_timeout);

        let to_publish = trigger.prune();
        assert!(to_publish.is_empty());
        assert_eq!(trigger.len(), 0);
    }

    #[test]
    fn test_update_on_expired_entry() {
        let trigger = MempoolPublishTrigger::new(test_config());
        trigger.update(BLOB_ID, 10);

        thread::sleep(test_config().share_timeout);

        trigger.prune();
        assert_eq!(trigger.update(BLOB_ID, 10), ShareState::Expired);
    }

    #[test]
    fn test_prune_publish_is_once() {
        let trigger = MempoolPublishTrigger::new(test_config());
        for _ in 0..5 {
            trigger.update(BLOB_ID, 10);
        }

        thread::sleep(test_config().share_timeout);

        assert_eq!(trigger.prune(), vec![BLOB_ID]);
        assert!(trigger.prune().is_empty());
    }

    #[test]
    fn test_concurrent_updates() {
        let assignations = 100;
        let trigger = Arc::new(MempoolPublishTrigger::new(test_config()));

        let mut handles = vec![];
        for _ in 0..assignations {
            let trigger_clone = Arc::clone(&trigger);
            handles.push(thread::spawn(move || {
                trigger_clone.update(BLOB_ID, assignations as u16)
            }));
        }

        let mut complete_count = 0;
        let mut incomplete_count = 0;
        for handle in handles {
            match handle.join().unwrap() {
                ShareState::Complete => complete_count += 1,
                ShareState::Incomplete => incomplete_count += 1,
                ShareState::Expired => panic!("Should not be expired"),
            }
        }

        assert_eq!(
            complete_count, 1,
            "Exactly one update should return Complete"
        );
        assert_eq!(
            incomplete_count,
            assignations - 1,
            "The rest should be Incomplete"
        );
        assert_eq!(
            trigger.get_count(&BLOB_ID),
            Some(assignations as u16),
            "Final count should match number of updates"
        );
    }
}
