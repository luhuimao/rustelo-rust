use crate::id;
use crate::ConfigState;
use soros_sdk::instruction::{AccountMeta, Instruction};
use soros_sdk::pubkey::Pubkey;
use soros_sdk::system_instruction;

/// Create a new, empty configuration account
pub fn create_account<T: ConfigState>(
    from_account_pubkey: &Pubkey,
    config_account_pubkey: &Pubkey,
    // lamports: u64,
    dif: u64,
) -> Instruction {
    system_instruction::create_account(
        from_account_pubkey,
        config_account_pubkey,
        // lamports,
        dif,
        T::max_space(),
        &id(),
    )
}

/// Store new data in a configuration account
pub fn store<T: ConfigState>(
    from_account_pubkey: &Pubkey,
    config_account_pubkey: &Pubkey,
    data: &T,
) -> Instruction {
    let account_metas = vec![
        AccountMeta::new(*from_account_pubkey, true),
        AccountMeta::new(*config_account_pubkey, true),
    ];
    Instruction::new(id(), data, account_metas)
}
