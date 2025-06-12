use std::{collections::HashMap, sync::Mutex};

use crate::{membership::Assignations, storage::MembershipStorage};

#[derive(Default)]
pub struct MockStorage {
    storage: Mutex<HashMap<u64, Assignations>>,
}

impl MembershipStorage for MockStorage {
    fn store(&self, block_number: u64, assignations: Assignations) {
        let mut storage = self.storage.lock().unwrap();
        storage.insert(block_number, assignations);
    }

    fn get(&self, block_number: u64) -> Option<Assignations> {
        self.storage.lock().unwrap().get(&block_number).cloned()
    }
}
