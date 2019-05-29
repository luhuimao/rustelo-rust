use bincode::deserialize;
use log::*;
use soros_runtime::bank::Bank;
use soros_sdk::genesis_block::GenesisBlock;
use soros_sdk::hash::{hash, Hash};
use soros_sdk::pubkey::Pubkey;
use soros_sdk::signature::{Keypair, KeypairUtil};
use soros_sdk::system_transaction::SystemTransaction;
use soros_storage_api::{StorageTransaction, ENTRIES_PER_SEGMENT};

fn get_storage_entry_height(bank: &Bank, account: &Pubkey) -> u64 {
    match bank.get_account(&account) {
        Some(storage_system_account) => {
            let state = deserialize(&storage_system_account.data);
            if let Ok(state) = state {
                let state: soros_storage_api::StorageProgramState = state;
                return state.entry_height;
            }
        }
        None => {
            info!("error in reading entry_height");
        }
    }
    0
}

fn get_storage_blockhash(bank: &Bank, account: &Pubkey) -> Hash {
    if let Some(storage_system_account) = bank.get_account(&account) {
        let state = deserialize(&storage_system_account.data);
        if let Ok(state) = state {
            let state: soros_storage_api::StorageProgramState = state;
            return state.hash;
        }
    }
    Hash::default()
}

#[test]
fn test_bank_storage() {
    let (genesis_block, alice) = GenesisBlock::new(1000);
    let bank = Bank::new(&genesis_block);

    let bob = Keypair::new();
    let jack = Keypair::new();
    let jill = Keypair::new();

    let x = 42;
    let blockhash = genesis_block.hash();
    let x2 = x * 2;
    let storage_blockhash = hash(&[x2]);

    bank.register_tick(&blockhash);

    bank.transfer(10, &alice, &jill.pubkey(), blockhash)
        .unwrap();

    bank.transfer(10, &alice, &bob.pubkey(), blockhash).unwrap();
    bank.transfer(10, &alice, &jack.pubkey(), blockhash)
        .unwrap();

    let tx = SystemTransaction::new_program_account(
        &alice,
        &bob.pubkey(),
        blockhash,
        1,
        4 * 1024,
        &soros_storage_api::id(),
        0,
    );

    bank.process_transaction(&tx).unwrap();

    let tx = StorageTransaction::new_advertise_recent_blockhash(
        &bob,
        storage_blockhash,
        blockhash,
        ENTRIES_PER_SEGMENT,
    );

    bank.process_transaction(&tx).unwrap();

    // TODO: This triggers a ProgramError(0, InvalidArgument). Why?
    // Oddly, the assertions below it succeed without running this code.
    //let entry_height = 0;
    //let tx = StorageTransaction::new_mining_proof(
    //    &jack,
    //    Hash::default(),
    //    blockhash,
    //    entry_height,
    //    Signature::default(),
    //);
    //let _result = bank.process_transaction(&tx).unwrap();

    assert_eq!(
        get_storage_entry_height(&bank, &bob.pubkey()),
        ENTRIES_PER_SEGMENT
    );
    assert_eq!(
        get_storage_blockhash(&bank, &bob.pubkey()),
        storage_blockhash
    );
}