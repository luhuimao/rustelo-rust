use serde::Serialize;
use soros_sdk::pubkey::Pubkey;

pub mod config_instruction;
pub mod config_processor;

const CONFIG_PROGRAM_ID: [u8; 32] = [
    3, 6, 74, 163, 0, 47, 116, 220, 200, 110, 67, 49, 15, 12, 5, 42, 248, 197, 218, 39, 246, 16,
    64, 25, 163, 35, 239, 160, 0, 0, 0, 0,
];

pub fn check_id(program_id: &Pubkey) -> bool {
    program_id.as_ref() == CONFIG_PROGRAM_ID
}

pub fn id() -> Pubkey {
    Pubkey::new(&CONFIG_PROGRAM_ID)
}

pub trait ConfigState: Serialize {
    /// Maximum space that the serialized representation will require
    fn max_space() -> u64;
}
