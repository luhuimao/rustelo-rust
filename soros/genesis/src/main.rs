//! A command-line executable for generating the chain's genesis block.

use clap::{crate_description, crate_name, crate_version, value_t_or_exit, App, Arg};
use soros::blocktree::create_new_ledger;
use soros_sdk::genesis_block::GenesisBlock;
use soros_sdk::signature::{read_keypair, KeypairUtil};
use std::error;

/**
 * Bootstrap leader gets two dif:
 * - 42 dif to use as stake
 * - One lamport to keep the node identity public key valid
 */
// pub const BOOTSTRAP_LEADER_LAMPORTS: u64 = 43;
pub const BOOTSTRAP_LEADER_DIF: u64 = 43;

fn main() -> Result<(), Box<dyn error::Error>> {
    // let default_bootstrap_leader_lamports = &BOOTSTRAP_LEADER_LAMPORTS.to_string();
    let default_bootstrap_leader_dif = &BOOTSTRAP_LEADER_DIF.to_string();
    let matches = App::new(crate_name!())
        .about(crate_description!())
        .version(crate_version!())
        .arg(
            Arg::with_name("bootstrap_leader_keypair_file")
                .short("b")
                .long("bootstrap-leader-keypair")
                .value_name("BOOTSTRAP LEADER KEYPAIR")
                .takes_value(true)
                .required(true)
                .help("Path to file containing the bootstrap leader's keypair"),
        )
        .arg(
            Arg::with_name("ledger_path")
                .short("l")
                .long("ledger")
                .value_name("DIR")
                .takes_value(true)
                .required(true)
                .help("Use directory as persistent ledger location"),
        )
        .arg(
            // Arg::with_name("lamports")
            Arg::with_name("dif")
                .short("t")
                // .long("lamports")
                .long("dif")
                // .value_name("LAMPORTS")
                .value_name("DIF")
                .takes_value(true)
                .required(true)
                // .help("Number of lamports to create in the mint"),
                .help("Number of dif to create in the mint"),
        )
        .arg(
            Arg::with_name("mint_keypair_file")
                .short("m")
                .long("mint")
                .value_name("MINT")
                .takes_value(true)
                .required(true)
                .help("Path to file containing keys of the mint"),
        )
        .arg(
            Arg::with_name("bootstrap_vote_keypair_file")
                .short("s")
                .long("bootstrap-vote-keypair")
                .value_name("BOOTSTRAP VOTE KEYPAIR")
                .takes_value(true)
                .required(true)
                .help("Path to file containing the bootstrap leader's staking keypair"),
        )
        .arg(
            // Arg::with_name("bootstrap_leader_lamports")
            Arg::with_name("bootstrap_leader_dif")
                // .long("bootstrap-leader-lamports")
                .long("bootstrap-leader-dif")
                // .value_name("LAMPORTS")
                .value_name("DIF")
                .takes_value(true)
                // .default_value(default_bootstrap_leader_lamports)
                .default_value(default_bootstrap_leader_dif)
                .required(true)
                // .help("Number of lamports to assign to the bootstrap leader"),
                .help("Number of dif to assign to the bootstrap leader"),
        )
        .get_matches();

    let bootstrap_leader_keypair_file = matches.value_of("bootstrap_leader_keypair_file").unwrap();
    let bootstrap_vote_keypair_file = matches.value_of("bootstrap_vote_keypair_file").unwrap();
    let ledger_path = matches.value_of("ledger_path").unwrap();
    let mint_keypair_file = matches.value_of("mint_keypair_file").unwrap();
    // let lamports = value_t_or_exit!(matches, "lamports", u64);
    let dif = value_t_or_exit!(matches, "dif", u64);
    // let bootstrap_leader_lamports = value_t_or_exit!(matches, "bootstrap_leader_lamports", u64);
    let bootstrap_leader_dif = value_t_or_exit!(matches, "bootstrap_leader_dif", u64);

    let bootstrap_leader_keypair = read_keypair(bootstrap_leader_keypair_file)?;
    let bootstrap_vote_keypair = read_keypair(bootstrap_vote_keypair_file)?;
    let mint_keypair = read_keypair(mint_keypair_file)?;

    let (mut genesis_block, _mint_keypair) = GenesisBlock::new_with_leader(
        // lamports,
        dif,
        &bootstrap_leader_keypair.pubkey(),
        // bootstrap_leader_lamports,
        bootstrap_leader_dif,
    );
    genesis_block.mint_id = mint_keypair.pubkey();
    genesis_block.bootstrap_leader_vote_account_id = bootstrap_vote_keypair.pubkey();
    genesis_block
        .native_instruction_processors
        .extend_from_slice(&[
            ("soros_budget_program".to_string(), soros_budget_api::id()),
            (
                "soros_storage_program".to_string(),
                soros_storage_api::id(),
            ),
            ("soros_token_program".to_string(), soros_token_api::id()),
            ("soros_config_program".to_string(), soros_config_api::id()),
            (
                "soros_exchange_program".to_string(),
                soros_exchange_api::id(),
            ),
        ]);

    create_new_ledger(ledger_path, &genesis_block)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use hashbrown::HashSet;
    use soros_sdk::pubkey::Pubkey;

    #[test]
    fn test_program_ids() {
        let system = Pubkey::new(&[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ]);
        let native_loader = "NativeLoader1111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let bpf_loader = "BPFLoader1111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let budget = "Budget1111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let stake = "Stake11111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let storage = "Storage111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let token = "Token11111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let vote = "Vote111111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let config = "Config1111111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();
        let exchange = "Exchange11111111111111111111111111111111111"
            .parse::<Pubkey>()
            .unwrap();

        assert_eq!(soros_sdk::system_program::id(), system);
        assert_eq!(soros_sdk::native_loader::id(), native_loader);
        assert_eq!(soros_sdk::bpf_loader::id(), bpf_loader);
        assert_eq!(soros_budget_api::id(), budget);
        assert_eq!(soros_stake_api::id(), stake);
        assert_eq!(soros_storage_api::id(), storage);
        assert_eq!(soros_token_api::id(), token);
        assert_eq!(soros_vote_api::id(), vote);
        assert_eq!(soros_config_api::id(), config);
        assert_eq!(soros_exchange_api::id(), exchange);
    }

    #[test]
    fn test_program_id_uniqueness() {
        let mut unique = HashSet::new();
        let ids = vec![
            soros_sdk::system_program::id(),
            soros_sdk::native_loader::id(),
            soros_sdk::bpf_loader::id(),
            soros_budget_api::id(),
            soros_storage_api::id(),
            soros_token_api::id(),
            soros_vote_api::id(),
            soros_config_api::id(),
            soros_exchange_api::id(),
        ];
        assert!(ids.into_iter().all(move |id| unique.insert(id)));
    }
}
