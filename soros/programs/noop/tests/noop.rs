use soros_runtime::bank::Bank;
use soros_runtime::loader_utils::load_program;
use soros_sdk::genesis_block::GenesisBlock;
use soros_sdk::native_loader;
use soros_sdk::transaction::Transaction;

#[test]
fn test_program_native_noop() {
    soros_logger::setup();

    let (genesis_block, mint_keypair) = GenesisBlock::new(50);
    let bank = Bank::new(&genesis_block);

    let program = "noop".as_bytes().to_vec();
    let program_id = load_program(&bank, &mint_keypair, &native_loader::id(), program);

    // Call user program
    let tx = Transaction::new(
        &mint_keypair,
        &[],
        &program_id,
        &1u8,
        bank.last_blockhash(),
        0,
    );
    bank.process_transaction(&tx).unwrap();
    assert_eq!(bank.get_signature_status(&tx.signatures[0]), Some(Ok(())));
}