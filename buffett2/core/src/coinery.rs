use crate::entry::Entry;
use buffett_crypto::hash::{hash, Hash};
use ring::rand::SystemRandom;
use buffett_crypto::signature::{Keypair, KeypairUtil};
use buffett_interface::pubkey::Pubkey;
use crate::system_transaction::SystemTransaction;
use crate::transaction::Transaction;
use untrusted::Input;

#[derive(Serialize, Deserialize, Debug)]
pub struct Mint {
    pub pkcs8: Vec<u8>,
    pubkey: Pubkey,
    pub tokens: i64,
}

impl Mint {
    pub fn new_with_pkcs8(tokens: i64, pkcs8: Vec<u8>) -> Self {
        let keypair =
            Keypair::from_pkcs8(Input::from(&pkcs8)).expect("from_pkcs8 in mint pub fn new");
        let pubkey = keypair.pubkey();
        Mint {
            pkcs8,
            pubkey,
            tokens,
        }
    }

    pub fn new(tokens: i64) -> Self {
        let rnd = SystemRandom::new();
        let pkcs8 = Keypair::generate_pkcs8(&rnd)
            .expect("generate_pkcs8 in mint pub fn new")
            .to_vec();
        Self::new_with_pkcs8(tokens, pkcs8)
    }

    pub fn seed(&self) -> Hash {
        hash(&self.pkcs8)
    }

    pub fn last_id(&self) -> Hash {
        self.create_entries()[1].id
    }

    pub fn keypair(&self) -> Keypair {
        Keypair::from_pkcs8(Input::from(&self.pkcs8)).expect("from_pkcs8 in mint pub fn keypair")
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    pub fn create_transactions(&self) -> Vec<Transaction> {
        let keypair = self.keypair();
        let tx = Transaction::system_move(&keypair, self.pubkey(), self.tokens, self.seed(), 0);
        vec![tx]
    }

    pub fn create_entries(&self) -> Vec<Entry> {
        let e0 = Entry::new(&self.seed(), 0, vec![]);
        let e1 = Entry::new(&e0.id, 0, self.create_transactions());
        vec![e0, e1]
    }
}

