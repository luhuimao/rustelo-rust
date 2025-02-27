#![feature(test)]

extern crate test;

use soros_runtime::bank::*;
use soros_sdk::account::Account;
use soros_sdk::genesis_block::GenesisBlock;
use soros_sdk::pubkey::Pubkey;
use std::sync::Arc;
use test::Bencher;

fn deposit_many(bank: &Bank, pubkeys: &mut Vec<Pubkey>, num: usize) {
    for t in 0..num {
        let pubkey = Pubkey::new_rand();
        let account = Account::new((t + 1) as u64, 0, &Account::default().owner);
        pubkeys.push(pubkey.clone());
        assert!(bank.get_account(&pubkey).is_none());
        bank.deposit(&pubkey, (t + 1) as u64);
        assert_eq!(bank.get_account(&pubkey).unwrap(), account);
    }
}

#[bench]
fn test_accounts_create(bencher: &mut Bencher) {
    let (genesis_block, _) = GenesisBlock::new(10_000);
    let bank0 = Bank::new_with_paths(&genesis_block, Some("bench_a0".to_string()));
    bencher.iter(|| {
        let mut pubkeys: Vec<Pubkey> = vec![];
        deposit_many(&bank0, &mut pubkeys, 1000);
    });
}

#[bench]
fn test_accounts_squash(bencher: &mut Bencher) {
    let (genesis_block, _) = GenesisBlock::new(100_000);
    let mut banks: Vec<Arc<Bank>> = Vec::with_capacity(10);
    banks.push(Arc::new(Bank::new_with_paths(
        &genesis_block,
        Some("bench_a1".to_string()),
    )));
    let mut pubkeys: Vec<Pubkey> = vec![];
    deposit_many(&banks[0], &mut pubkeys, 250000);
    banks[0].freeze();
    // Measures the performance of the squash operation merging the accounts
    // with the majority of the accounts present in the parent bank that is
    // moved over to this bank.
    bencher.iter(|| {
        banks.push(Arc::new(Bank::new_from_parent(
            &banks[0],
            &Pubkey::default(),
            1u64,
        )));
        for accounts in 0..10000 {
            banks[1].deposit(&pubkeys[accounts], (accounts + 1) as u64);
        }
        banks[1].squash();
    });
}
