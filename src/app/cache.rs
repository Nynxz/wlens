//! Decoded images by source, with LRU eviction under a byte budget.

use super::*;

pub(super) enum Slot {
    Pending,
    Ready(Arc<Decoded>),
}

/// Decoded images by source, with LRU eviction under a byte budget.
#[derive(Default)]
pub(super) struct Cache {
    pub(super) slots: HashMap<Source, Slot>,
    pub(super) lru: VecDeque<Source>,
    pub(super) bytes: usize,
}

impl Cache {
    pub(super) fn get(&mut self, p: &Source) -> Option<&Slot> {
        if let Some(i) = self.lru.iter().position(|x| x == p) {
            let k = self.lru.remove(i).unwrap();
            self.lru.push_back(k);
        }
        self.slots.get(p)
    }

    pub(super) fn insert(
        &mut self,
        p: Source,
        img: Arc<Decoded>,
        budget: usize,
        keep: Option<&Source>,
    ) {
        self.remove(&p);
        self.bytes += img.bytes();
        self.slots.insert(p.clone(), Slot::Ready(img));
        self.lru.push_back(p);
        while self.bytes > budget {
            let Some(victim) = self.lru.iter().position(|x| Some(x) != keep) else {
                break;
            };
            let victim = self.lru.remove(victim).unwrap();
            self.remove(&victim);
        }
    }

    pub(super) fn remove(&mut self, p: &Source) {
        if let Some(Slot::Ready(img)) = self.slots.remove(p) {
            self.bytes -= img.bytes();
            self.lru.retain(|x| x != p);
        }
    }
}
