use buffett_crypto::hash::{hash, hashv, Hash};

pub struct Poh {
    last_hash: Hash,
    num_hashes: u64,
}

#[derive(Debug)]
pub struct PohEntry {
    pub num_hashes: u64,
    pub id: Hash,
    pub mixin: Option<Hash>,
}

impl Poh {
    pub fn new(last_hash: Hash) -> Self {
        Poh {
            last_hash,
            num_hashes: 0,
        }
    }

    pub fn hash(&mut self) {
        self.last_hash = hash(&self.last_hash.as_ref());
        self.num_hashes += 1;
    }

    pub fn record(&mut self, mixin: Hash) -> PohEntry {
        let num_hashes = self.num_hashes + 1;
        self.last_hash = hashv(&[&self.last_hash.as_ref(), &mixin.as_ref()]);

        self.num_hashes = 0;

        PohEntry {
            num_hashes,
            id: self.last_hash,
            mixin: Some(mixin),
        }
    }

    // emissions of Ticks (i.e. PohEntries without a mixin) allows
    //  validators to parallelize the work of catching up
    pub fn tick(&mut self) -> PohEntry {
        self.hash();

        let num_hashes = self.num_hashes;
        self.num_hashes = 0;

        PohEntry {
            num_hashes,
            id: self.last_hash,
            mixin: None,
        }
    }
}

#[cfg(test)]
pub fn verify(initial: Hash, entries: &[PohEntry]) -> bool {
    let mut last_hash = initial;

    for entry in entries {
        assert!(entry.num_hashes != 0);
        for _ in 1..entry.num_hashes {
            last_hash = hash(&last_hash.as_ref());
        }
        let id = match entry.mixin {
            Some(mixin) => hashv(&[&last_hash.as_ref(), &mixin.as_ref()]),
            None => hash(&last_hash.as_ref()),
        };
        if id != entry.id {
            return false;
        }
        last_hash = id;
    }

    true
}

