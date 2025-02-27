#[macro_use]
extern crate log;
extern crate bincode;
extern crate chrono;
extern crate serde_json;
extern crate buffett;
extern crate buffett_program_interface;

use buffett::crdt::{Crdt, Node, NodeInfo};
use buffett::entry::Entry;
use buffett::fullnode::{Fullnode, FullnodeReturnType};
use buffett::hash::Hash;
use buffett::ledger::{read_ledger, LedgerWriter};
use buffett::logger;
use buffett::mint::Mint;
use buffett::ncp::Ncp;
use buffett::result;
use buffett::service::Service;
use buffett::signature::{Keypair, KeypairUtil};
use buffett::thin_client::ThinClient;
use buffett::timing::{duration_as_ms, duration_as_s};
use buffett::window::{default_window, WINDOW_SIZE};
use buffett_program_interface::pubkey::Pubkey;
use std::env;
use std::fs::{copy, create_dir_all, remove_dir_all};
use std::net::UdpSocket;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::thread::sleep;
use std::thread::Builder;
use std::time::{Duration, Instant};

fn make_spy_node(leader: &NodeInfo) -> (Ncp, Arc<RwLock<Crdt>>, Pubkey) {
    let exit = Arc::new(AtomicBool::new(false));
    let mut spy = Node::new_localhost();
    let me = spy.info.id.clone();
    spy.info.contact_info.tvu = spy.sockets.replicate[0].local_addr().unwrap();
    spy.info.contact_info.rpu = spy.sockets.transaction[0].local_addr().unwrap();
    let mut spy_crdt = Crdt::new(spy.info).expect("Crdt::new");
    spy_crdt.insert(&leader);
    spy_crdt.set_leader(leader.id);
    let spy_crdt_ref = Arc::new(RwLock::new(spy_crdt));
    let spy_window = Arc::new(RwLock::new(default_window()));
    let ncp = Ncp::new(
        &spy_crdt_ref,
        spy_window,
        None,
        spy.sockets.gossip,
        exit.clone(),
    );

    (ncp, spy_crdt_ref, me)
}

fn converge(leader: &NodeInfo, num_nodes: usize) -> Vec<NodeInfo> {
    //lets spy on the network
    let (ncp, spy_ref, _) = make_spy_node(leader);

    //wait for the network to converge
    let mut converged = false;
    let mut rv = vec![];
    for _ in 0..30 {
        let num = spy_ref.read().unwrap().convergence();
        let mut v = spy_ref.read().unwrap().get_valid_peers();
        if num >= num_nodes as u64 && v.len() >= num_nodes {
            rv.append(&mut v);
            converged = true;
            break;
        }
        sleep(Duration::new(1, 0));
    }
    assert!(converged);
    ncp.close().unwrap();
    rv
}

fn tmp_ledger_path(name: &str) -> String {
    let keypair = Keypair::new();

    format!("/tmp/tmp-ledger-{}-{}", name, keypair.pubkey())
}

fn genesis(name: &str, num: i64) -> (Mint, String, Vec<Entry>) {
    let mint = Mint::new(num);

    let path = tmp_ledger_path(name);
    let mut writer = LedgerWriter::open(&path, true).unwrap();

    let entries = mint.create_entries();
    writer.write_entries(entries.clone()).unwrap();

    (mint, path, entries)
}

fn tmp_copy_ledger(from: &str, name: &str) -> String {
    let tostr = tmp_ledger_path(name);

    {
        let to = Path::new(&tostr);
        let from = Path::new(&from);

        create_dir_all(to).unwrap();

        copy(from.join("data"), to.join("data")).unwrap();
        copy(from.join("index"), to.join("index")).unwrap();
    }

    tostr
}

fn make_tiny_test_entries(start_hash: Hash, num: usize) -> Vec<Entry> {
    let mut id = start_hash;
    let mut num_hashes = 0;
    (0..num)
        .map(|_| Entry::new_mut(&mut id, &mut num_hashes, vec![]))
        .collect()
}

#[test]
fn test_multi_node_ledger_window() -> result::Result<()> {
    logger::setup();

    let leader_keypair = Keypair::new();
    let leader_pubkey = leader_keypair.pubkey().clone();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_data = leader.info.clone();
    let bob_pubkey = Keypair::new().pubkey();
    let mut ledger_paths = Vec::new();

    let (alice, leader_ledger_path, _) = genesis("multi_node_ledger_window", 10_000);
    ledger_paths.push(leader_ledger_path.clone());

    // make a copy at zero
    let zero_ledger_path = tmp_copy_ledger(&leader_ledger_path, "multi_node_ledger_window");
    ledger_paths.push(zero_ledger_path.clone());

    // write a bunch more ledger into leader's ledger, this should populate his window
    // and force him to respond to repair from the ledger window
    {
        let entries = make_tiny_test_entries(alice.last_id(), WINDOW_SIZE as usize);
        let mut writer = LedgerWriter::open(&leader_ledger_path, false).unwrap();

        writer.write_entries(entries).unwrap();
    }

    let leader = Fullnode::new(
        leader,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        None,
    );

    // Send leader some tokens to vote
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &leader_pubkey, 500, None).unwrap();
    info!("leader balance {}", leader_balance);

    // start up another validator from zero, converge and then check
    // balances
    let keypair = Keypair::new();
    let validator_pubkey = keypair.pubkey().clone();
    let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
    let validator_data = validator.info.clone();
    let validator = Fullnode::new(
        validator,
        &zero_ledger_path,
        keypair,
        Some(leader_data.contact_info.ncp),
        false,
        None,
    );

    // Send validator some tokens to vote
    let validator_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &validator_pubkey, 500, None).unwrap();
    info!("leader balance {}", validator_balance);

    // contains the leader and new node
    info!("converging....");
    let _servers = converge(&leader_data, 2);
    info!("converged.");

    // another transaction with leader
    let bob_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 1, None).unwrap();
    info!("bob balance on leader {}", bob_balance);
    let mut checks = 1;
    loop {
        let mut client = mk_client(&validator_data);
        let bal = client.poll_get_balance(&bob_pubkey);
        info!(
            "bob balance on validator {:?} after {} checks...",
            bal, checks
        );
        if bal.unwrap_or(0) == bob_balance {
            break;
        }
        checks += 1;
    }
    info!("done!");

    validator.close()?;
    leader.close()?;

    for path in ledger_paths {
        remove_dir_all(path).unwrap();
    }

    Ok(())
}

#[test]
#[ignore]
fn test_multi_node_validator_catchup_from_zero() -> result::Result<()> {
    logger::setup();
    const N: usize = 5;
    trace!("test_multi_node_validator_catchup_from_zero");
    let leader_keypair = Keypair::new();
    let leader_pubkey = leader_keypair.pubkey().clone();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_data = leader.info.clone();
    let bob_pubkey = Keypair::new().pubkey();
    let mut ledger_paths = Vec::new();

    let (alice, leader_ledger_path, _) = genesis("multi_node_validator_catchup_from_zero", 10_000);
    ledger_paths.push(leader_ledger_path.clone());

    let zero_ledger_path = tmp_copy_ledger(
        &leader_ledger_path,
        "multi_node_validator_catchup_from_zero",
    );
    ledger_paths.push(zero_ledger_path.clone());

    let server = Fullnode::new(
        leader,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        None,
    );

    // Send leader some tokens to vote
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &leader_pubkey, 500, None).unwrap();
    info!("leader balance {}", leader_balance);

    let mut nodes = vec![server];
    for _ in 0..N {
        let keypair = Keypair::new();
        let validator_pubkey = keypair.pubkey().clone();
        let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
        let ledger_path = tmp_copy_ledger(
            &leader_ledger_path,
            "multi_node_validator_catchup_from_zero_validator",
        );
        ledger_paths.push(ledger_path.clone());

        // Send each validator some tokens to vote
        let validator_balance =
            send_tx_and_retry_get_balance(&leader_data, &alice, &validator_pubkey, 500, None)
                .unwrap();
        info!("validator balance {}", validator_balance);

        let mut val = Fullnode::new(
            validator,
            &ledger_path,
            keypair,
            Some(leader_data.contact_info.ncp),
            false,
            None,
        );
        nodes.push(val);
    }
    let servers = converge(&leader_data, N + 1);
    //contains the leader addr as well
    assert_eq!(servers.len(), N + 1);
    //verify leader can do transfer
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, None).unwrap();
    assert_eq!(leader_balance, 500);
    //verify validator has the same balance
    let mut success = 0usize;
    for server in servers.iter() {
        info!("0server: {}", server.id);
        let mut client = mk_client(server);
        if let Ok(bal) = client.poll_get_balance(&bob_pubkey) {
            info!("validator balance {}", bal);
            if bal == leader_balance {
                success += 1;
            }
        }
    }
    assert_eq!(success, servers.len());

    success = 0;
    // start up another validator from zero, converge and then check everyone's
    // balances
    let keypair = Keypair::new();
    let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
    let val = Fullnode::new(
        validator,
        &zero_ledger_path,
        keypair,
        Some(leader_data.contact_info.ncp),
        false,
        None,
    );
    nodes.push(val);
    //contains the leader and new node
    let servers = converge(&leader_data, N + 2);

    let mut leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, None).unwrap();
    info!("leader balance {}", leader_balance);
    loop {
        let mut client = mk_client(&leader_data);
        leader_balance = client.poll_get_balance(&bob_pubkey)?;
        if leader_balance == 1000 {
            break;
        }
        sleep(Duration::from_millis(300));
    }
    assert_eq!(leader_balance, 1000);

    for server in servers.iter() {
        let mut client = mk_client(server);
        info!("1server: {}", server.id);
        for _ in 0..15 {
            if let Ok(bal) = client.poll_get_balance(&bob_pubkey) {
                info!("validator balance {}", bal);
                if bal == leader_balance {
                    success += 1;
                    break;
                }
            }
            sleep(Duration::from_millis(500));
        }
    }
    assert_eq!(success, servers.len());

    for node in nodes {
        node.close()?;
    }
    for path in ledger_paths {
        remove_dir_all(path).unwrap();
    }

    Ok(())
}

#[test]
#[ignore]
fn test_multi_node_basic() {
    logger::setup();
    const N: usize = 5;
    trace!("test_multi_node_basic");
    let leader_keypair = Keypair::new();
    let leader_pubkey = leader_keypair.pubkey().clone();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_data = leader.info.clone();
    let bob_pubkey = Keypair::new().pubkey();
    let mut ledger_paths = Vec::new();

    let (alice, leader_ledger_path, _) = genesis("multi_node_basic", 10_000);
    ledger_paths.push(leader_ledger_path.clone());
    let server = Fullnode::new(
        leader,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        None,
    );

    // Send leader some tokens to vote
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &leader_pubkey, 500, None).unwrap();
    info!("leader balance {}", leader_balance);

    let mut nodes = vec![server];
    for _ in 0..N {
        let keypair = Keypair::new();
        let validator_pubkey = keypair.pubkey().clone();
        let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
        let ledger_path = tmp_copy_ledger(&leader_ledger_path, "multi_node_basic");
        ledger_paths.push(ledger_path.clone());

        // Send each validator some tokens to vote
        let validator_balance =
            send_tx_and_retry_get_balance(&leader_data, &alice, &validator_pubkey, 500, None)
                .unwrap();
        info!("validator balance {}", validator_balance);

        let val = Fullnode::new(
            validator,
            &ledger_path,
            keypair,
            Some(leader_data.contact_info.ncp),
            false,
            None,
        );
        nodes.push(val);
    }
    let servers = converge(&leader_data, N + 1);
    //contains the leader addr as well
    assert_eq!(servers.len(), N + 1);
    //verify leader can do transfer
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, None).unwrap();
    assert_eq!(leader_balance, 500);
    //verify validator has the same balance
    let mut success = 0usize;
    for server in servers.iter() {
        let mut client = mk_client(server);
        if let Ok(bal) = client.poll_get_balance(&bob_pubkey) {
            trace!("validator balance {}", bal);
            if bal == leader_balance {
                success += 1;
            }
        }
    }
    assert_eq!(success, servers.len());

    for node in nodes {
        node.close().unwrap();
    }
    for path in ledger_paths {
        remove_dir_all(path).unwrap();
    }
}

#[test]
fn test_boot_validator_from_file() -> result::Result<()> {
    logger::setup();
    let leader_keypair = Keypair::new();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let bob_pubkey = Keypair::new().pubkey();
    let (alice, leader_ledger_path, _) = genesis("boot_validator_from_file", 100_000);
    let mut ledger_paths = Vec::new();
    ledger_paths.push(leader_ledger_path.clone());

    let leader_data = leader.info.clone();
    let leader_fullnode = Fullnode::new(
        leader,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        None,
    );
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, Some(500)).unwrap();
    assert_eq!(leader_balance, 500);
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, Some(1000)).unwrap();
    assert_eq!(leader_balance, 1000);

    let keypair = Keypair::new();
    let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
    let validator_data = validator.info.clone();
    let ledger_path = tmp_copy_ledger(&leader_ledger_path, "boot_validator_from_file");
    ledger_paths.push(ledger_path.clone());
    let val_fullnode = Fullnode::new(
        validator,
        &ledger_path,
        keypair,
        Some(leader_data.contact_info.ncp),
        false,
        None,
    );
    let mut client = mk_client(&validator_data);
    let getbal = retry_get_balance(&mut client, &bob_pubkey, Some(leader_balance));
    assert!(getbal == Some(leader_balance));

    val_fullnode.close()?;
    leader_fullnode.close()?;
    for path in ledger_paths {
        remove_dir_all(path)?;
    }

    Ok(())
}

fn create_leader(ledger_path: &str) -> (NodeInfo, Fullnode) {
    let leader_keypair = Keypair::new();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_data = leader.info.clone();
    let leader_fullnode = Fullnode::new(leader, &ledger_path, leader_keypair, None, false, None);
    (leader_data, leader_fullnode)
}

#[test]
fn test_leader_restart_validator_start_from_old_ledger() -> result::Result<()> {
    // this test verifies that a freshly started leader makes his ledger available
    //    in the repair window to validators that are started with an older
    //    ledger (currently up to WINDOW_SIZE entries)
    logger::setup();

    let (alice, ledger_path, _) = genesis(
        "leader_restart_validator_start_from_old_ledger",
        100_000 + 500 * buffett::window_service::MAX_REPAIR_BACKOFF as i64,
    );
    let bob_pubkey = Keypair::new().pubkey();

    let (leader_data, leader_fullnode) = create_leader(&ledger_path);

    // lengthen the ledger
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, Some(500)).unwrap();
    assert_eq!(leader_balance, 500);

    // create a "stale" ledger by copying current ledger
    let stale_ledger_path = tmp_copy_ledger(
        &ledger_path,
        "leader_restart_validator_start_from_old_ledger",
    );

    // restart the leader
    leader_fullnode.close()?;
    let (leader_data, leader_fullnode) = create_leader(&ledger_path);

    // lengthen the ledger
    let leader_balance =
        send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, Some(1000)).unwrap();
    assert_eq!(leader_balance, 1000);

    // restart the leader
    leader_fullnode.close()?;
    let (leader_data, leader_fullnode) = create_leader(&ledger_path);

    // start validator from old ledger
    let keypair = Keypair::new();
    let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
    let validator_data = validator.info.clone();

    let val_fullnode = Fullnode::new(
        validator,
        &stale_ledger_path,
        keypair,
        Some(leader_data.contact_info.ncp),
        false,
        None,
    );

    // trigger broadcast, validator should catch up from leader, whose window contains
    //   the entries missing from the stale ledger
    //   send requests so the validator eventually sees a gap and requests a repair
    let mut expected = 1500;
    let mut client = mk_client(&validator_data);
    for _ in 0..buffett::window_service::MAX_REPAIR_BACKOFF {
        let leader_balance =
            send_tx_and_retry_get_balance(&leader_data, &alice, &bob_pubkey, 500, Some(expected))
                .unwrap();
        assert_eq!(leader_balance, expected);

        let getbal = retry_get_balance(&mut client, &bob_pubkey, Some(leader_balance));
        if getbal == Some(leader_balance) {
            break;
        }

        expected += 500;
    }
    let getbal = retry_get_balance(&mut client, &bob_pubkey, Some(expected));
    assert_eq!(getbal, Some(expected));

    val_fullnode.close()?;
    leader_fullnode.close()?;
    remove_dir_all(ledger_path)?;
    remove_dir_all(stale_ledger_path)?;

    Ok(())
}

//TODO: this test will run a long time so it's disabled for CI
#[test]
#[ignore]
fn test_multi_node_dynamic_network() {
    logger::setup();
    assert!(cfg!(feature = "test"));
    let key = "BUFFETT_DYNAMIC_NODES";
    let num_nodes: usize = match env::var(key) {
        Ok(val) => val
            .parse()
            .expect(&format!("env var {} is not parse-able as usize", key)),
        Err(_) => 120,
    };

    let leader_keypair = Keypair::new();
    let leader_pubkey = leader_keypair.pubkey().clone();
    let leader = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let bob_pubkey = Keypair::new().pubkey();
    let (alice, leader_ledger_path, _) = genesis("multi_node_dynamic_network", 10_000_000);

    let mut ledger_paths = Vec::new();
    ledger_paths.push(leader_ledger_path.clone());

    let alice_arc = Arc::new(RwLock::new(alice));
    let leader_data = leader.info.clone();

    let server = Fullnode::new(
        leader,
        &leader_ledger_path,
        leader_keypair,
        None,
        true,
        None,
    );

    // Send leader some tokens to vote
    let leader_balance = send_tx_and_retry_get_balance(
        &leader_data,
        &alice_arc.read().unwrap(),
        &leader_pubkey,
        500,
        None,
    ).unwrap();
    info!("leader balance {}", leader_balance);

    info!("{} LEADER", leader_data.id);
    let leader_balance = retry_send_tx_and_retry_get_balance(
        &leader_data,
        &alice_arc.read().unwrap(),
        &bob_pubkey,
        Some(500),
    ).unwrap();
    assert_eq!(leader_balance, 500);
    let leader_balance = retry_send_tx_and_retry_get_balance(
        &leader_data,
        &alice_arc.read().unwrap(),
        &bob_pubkey,
        Some(1000),
    ).unwrap();
    assert_eq!(leader_balance, 1000);

    let t1: Vec<_> = (0..num_nodes)
        .into_iter()
        .map(|n| {
            let leader_data = leader_data.clone();
            let alice_clone = alice_arc.clone();
            Builder::new()
                .name("keypair-thread".to_string())
                .spawn(move || {
                    info!("Spawned thread {}", n);
                    let keypair = Keypair::new();
                    //send some tokens to the new validators
                    let bal = retry_send_tx_and_retry_get_balance(
                        &leader_data,
                        &alice_clone.read().unwrap(),
                        &keypair.pubkey(),
                        Some(500),
                    );
                    assert_eq!(bal, Some(500));
                    info!("sent balance to[{}/{}] {}", n, num_nodes, keypair.pubkey());
                    keypair
                }).unwrap()
        }).collect();

    info!("Waiting for keypairs to be created");
    let keypairs: Vec<_> = t1.into_iter().map(|t| t.join().unwrap()).collect();
    info!("keypairs created");

    let t2: Vec<_> = keypairs
        .into_iter()
        .map(|keypair| {
            let leader_data = leader_data.clone();
            let ledger_path = tmp_copy_ledger(&leader_ledger_path, "multi_node_dynamic_network");
            ledger_paths.push(ledger_path.clone());
            Builder::new()
                .name("validator-launch-thread".to_string())
                .spawn(move || {
                    let validator = Node::new_localhost_with_pubkey(keypair.pubkey());
                    let rd = validator.info.clone();
                    info!("starting {} {}", keypair.pubkey(), rd.id);
                    let val = Fullnode::new(
                        validator,
                        &ledger_path,
                        keypair,
                        Some(leader_data.contact_info.ncp),
                        true,
                        None,
                    );
                    (rd, val)
                }).unwrap()
        }).collect();

    let mut validators: Vec<_> = t2.into_iter().map(|t| t.join().unwrap()).collect();

    let mut client = mk_client(&leader_data);
    let mut last_finality = client.get_finality();
    info!("Last finality {}", last_finality);
    let start = Instant::now();
    let mut consecutive_success = 0;
    let mut expected_balance = leader_balance;
    for i in 0..std::cmp::max(20, num_nodes) {
        trace!("getting leader last_id");
        let last_id = client.get_last_id();
        trace!("executing leader transfer");
        let sig = client
            .transfer(
                500,
                &alice_arc.read().unwrap().keypair(),
                bob_pubkey,
                &last_id,
            ).unwrap();

        expected_balance += 500;

        let e = client.poll_for_signature(&sig);
        assert!(e.is_ok(), "err: {:?}", e);

        let now = Instant::now();
        let mut finality = client.get_finality();

        // Need this to make sure the finality is updated
        // (i.e. the node is not returning stale value)
        while last_finality == finality {
            finality = client.get_finality();
        }

        while duration_as_ms(&now.elapsed()) < finality as u64 {
            sleep(Duration::from_millis(100));
            finality = client.get_finality()
        }

        last_finality = finality;

        let balance = retry_get_balance(&mut client, &bob_pubkey, Some(expected_balance));
        assert_eq!(balance, Some(expected_balance));
        consecutive_success += 1;

        info!(
            "SUCCESS[{}] balance: {}, finality: {} ms",
            i, expected_balance, last_finality,
        );

        if consecutive_success == 10 {
            info!("Took {} s to converge", duration_as_s(&start.elapsed()),);
            info!("Verifying signature of the last transaction in the validators");

            let mut num_nodes_behind = 0i64;
            validators.retain(|server| {
                let mut client = mk_client(&server.0);
                trace!("{} checking signature", server.0.id);
                num_nodes_behind += if client.check_signature(&sig) { 0 } else { 1 };
                server.1.exit();
                true
            });

            info!(
                "Validators lagging: {}/{}",
                num_nodes_behind,
                validators.len(),
            );
            break;
        }
    }

    assert_eq!(consecutive_success, 10);
    for (_, node) in &validators {
        node.exit();
    }
    server.exit();
    for (_, node) in validators {
        node.join().unwrap();
    }
    server.join().unwrap();

    for path in ledger_paths {
        remove_dir_all(path).unwrap();
    }
}

#[test]
#[ignore]
fn test_leader_to_validator_transition() {
    logger::setup();
    let leader_rotation_interval = 20;

    // Make a dummy address to be the sink for this test's mock transactions
    let bob_pubkey = Keypair::new().pubkey();

    // Initialize the leader ledger. Make a mint and a genesis entry
    // in the leader ledger
    let (mint, leader_ledger_path, entries) = genesis(
        "test_leader_to_validator_transition",
        (3 * leader_rotation_interval) as i64,
    );

    let genesis_height = entries.len() as u64;

    // Start the leader node
    let leader_keypair = Keypair::new();
    let leader_node = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_info = leader_node.info.clone();
    let mut leader = Fullnode::new(
        leader_node,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        Some(leader_rotation_interval),
    );

    // Set the next leader to be Bob
    leader.set_scheduled_leader(bob_pubkey, leader_rotation_interval);

    // Make an extra node for our leader to broadcast to,
    // who won't vote and mess with our leader's entry count
    let (ncp, spy_node, _) = make_spy_node(&leader_info);

    // Wait for the leader to see the spy node
    let mut converged = false;
    for _ in 0..30 {
        let num = spy_node.read().unwrap().convergence();
        let mut v: Vec<NodeInfo> = spy_node.read().unwrap().get_valid_peers();
        // There's only one person excluding the spy node (the leader) who should see
        // two nodes on the network
        if num >= 2 as u64 && v.len() >= 1 {
            converged = true;
            break;
        }
        sleep(Duration::new(1, 0));
    }

    assert!(converged);

    let extra_transactions = std::cmp::max(leader_rotation_interval / 3, 1);

    // Push leader "extra_transactions" past leader_rotation_interval entry height,
    // make sure the leader stops.
    assert!(genesis_height < leader_rotation_interval);
    for i in genesis_height..(leader_rotation_interval + extra_transactions) {
        if i < leader_rotation_interval {
            // Poll to see that the bank state is updated after every transaction
            // to ensure that each transaction is packaged as a single entry,
            // so that we can be sure leader rotation is triggered
            let expected_balance = std::cmp::min(
                leader_rotation_interval - genesis_height,
                i - genesis_height + 1,
            );
            let result = send_tx_and_retry_get_balance(
                &leader_info,
                &mint,
                &bob_pubkey,
                1,
                Some(expected_balance as i64),
            );

            assert_eq!(result, Some(expected_balance as i64))
        } else {
            // After leader_rotation_interval entries, we don't care about the
            // number of entries generated by these transactions. These are just
            // here for testing to make sure the leader stopped at the correct point.
            send_tx_and_retry_get_balance(&leader_info, &mint, &bob_pubkey, 1, None);
        }
    }

    // Wait for leader to shut down tpu and restart tvu
    match leader.handle_role_transition().unwrap() {
        Some(FullnodeReturnType::LeaderRotation) => (),
        _ => panic!("Expected reason for exit to be leader rotation"),
    }

    // Query now validator to make sure that he has the proper balances in his bank
    // after the transition, even though we submitted "extra_transactions"
    // transactions earlier
    let mut leader_client = mk_client(&leader_info);

    let expected_bal = leader_rotation_interval - genesis_height;
    let bal = leader_client
        .poll_get_balance(&bob_pubkey)
        .expect("Expected success when polling newly transitioned validator for balance")
        as u64;

    assert_eq!(bal, expected_bal);

    // Shut down
    ncp.close().unwrap();
    leader.close().unwrap();
    remove_dir_all(leader_ledger_path).unwrap();
}

#[test]
#[ignore]
fn test_leader_validator_basic() {
    logger::setup();
    let leader_rotation_interval = 10;

    // Account that will be the sink for all the test's transactions
    let bob_pubkey = Keypair::new().pubkey();

    // Make a mint and a genesis entry in the leader ledger
    let (mint, leader_ledger_path, genesis_entries) =
        genesis("test_leader_validator_basic", 10_000);
    let genesis_height = genesis_entries.len();

    // Initialize the leader ledger
    let mut ledger_paths = Vec::new();
    ledger_paths.push(leader_ledger_path.clone());

    // Create the leader fullnode
    let leader_keypair = Keypair::new();
    let leader_node = Node::new_localhost_with_pubkey(leader_keypair.pubkey());
    let leader_info = leader_node.info.clone();

    let mut leader = Fullnode::new(
        leader_node,
        &leader_ledger_path,
        leader_keypair,
        None,
        false,
        Some(leader_rotation_interval),
    );

    // Send leader some tokens to vote
    send_tx_and_retry_get_balance(&leader_info, &mint, &leader_info.id, 500, None).unwrap();

    // Start the validator node
    let validator_ledger_path = tmp_copy_ledger(&leader_ledger_path, "test_leader_validator_basic");
    let validator_keypair = Keypair::new();
    let validator_node = Node::new_localhost_with_pubkey(validator_keypair.pubkey());
    let validator_info = validator_node.info.clone();
    let mut validator = Fullnode::new(
        validator_node,
        &validator_ledger_path,
        validator_keypair,
        Some(leader_info.contact_info.ncp),
        false,
        Some(leader_rotation_interval),
    );

    ledger_paths.push(validator_ledger_path.clone());

    // Set the leader schedule for the validator and leader
    let my_leader_begin_epoch = 2;
    for i in 0..my_leader_begin_epoch {
        validator.set_scheduled_leader(leader_info.id, leader_rotation_interval * i);
        leader.set_scheduled_leader(leader_info.id, leader_rotation_interval * i);
    }
    validator.set_scheduled_leader(
        validator_info.id,
        my_leader_begin_epoch * leader_rotation_interval,
    );
    leader.set_scheduled_leader(
        validator_info.id,
        my_leader_begin_epoch * leader_rotation_interval,
    );

    // Wait for convergence
    let servers = converge(&leader_info, 2);
    assert_eq!(servers.len(), 2);

    // Send transactions to the leader
    let extra_transactions = std::cmp::max(leader_rotation_interval / 3, 1);
    let total_transactions_to_send =
        my_leader_begin_epoch * leader_rotation_interval + extra_transactions;

    // Push "extra_transactions" past leader_rotation_interval entry height,
    // make sure the validator stops.
    for _ in genesis_height as u64..total_transactions_to_send {
        send_tx_and_retry_get_balance(&leader_info, &mint, &bob_pubkey, 1, None);
    }

    // Wait for validator to shut down tvu and restart tpu
    match validator.handle_role_transition().unwrap() {
        Some(FullnodeReturnType::LeaderRotation) => (),
        _ => panic!("Expected reason for exit to be leader rotation"),
    }

    // TODO: We ignore this test for now b/c there's a chance here that the crdt
    // in the new leader calls the dummy sequence of update_leader -> top_leader()
    // (see the TODOs in those functions) during gossip and sets the leader back
    // to the old leader, which causes a panic from an assertion failure in crdt broadcast(),
    // specifically: assert!(me.leader_id != v.id). We can enable this test once we have real
    // leader scheduling

    // Wait for the leader to shut down tpu and restart tvu
    match leader.handle_role_transition().unwrap() {
        Some(FullnodeReturnType::LeaderRotation) => (),
        _ => panic!("Expected reason for exit to be leader rotation"),
    }

    // Shut down
    validator.close().unwrap();
    leader.close().unwrap();

    // Check the ledger of the validator to make sure the entry height is correct
    // and that the old leader and the new leader's ledgers agree up to the point
    // of leader rotation
    let validator_entries =
        read_ledger(&validator_ledger_path, true).expect("Expected parsing of validator ledger");
    let leader_entries =
        read_ledger(&validator_ledger_path, true).expect("Expected parsing of leader ledger");

    for (v, l) in validator_entries.zip(leader_entries) {
        assert_eq!(
            v.expect("expected valid validator entry"),
            l.expect("expected valid leader entry")
        );
    }

    for path in ledger_paths {
        remove_dir_all(path).unwrap();
    }
}

fn mk_client(leader: &NodeInfo) -> ThinClient {
    let requests_socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    requests_socket
        .set_read_timeout(Some(Duration::new(1, 0)))
        .unwrap();
    let transactions_socket = UdpSocket::bind("0.0.0.0:0").unwrap();
    assert!(Crdt::is_valid_address(&leader.contact_info.rpu));
    assert!(Crdt::is_valid_address(&leader.contact_info.tpu));
    ThinClient::new(
        leader.contact_info.rpu,
        requests_socket,
        leader.contact_info.tpu,
        transactions_socket,
    )
}

fn retry_get_balance(
    client: &mut ThinClient,
    bob_pubkey: &Pubkey,
    expected: Option<i64>,
) -> Option<i64> {
    const LAST: usize = 30;
    for run in 0..(LAST + 1) {
        let out = client.poll_get_balance(bob_pubkey);
        if expected.is_none() || run == LAST {
            return out.ok().clone();
        }
        trace!("retry_get_balance[{}] {:?} {:?}", run, out, expected);
        if let (Some(e), Ok(o)) = (expected, out) {
            if o == e {
                return Some(o);
            }
        }
    }
    None
}

fn send_tx_and_retry_get_balance(
    leader: &NodeInfo,
    alice: &Mint,
    bob_pubkey: &Pubkey,
    transfer_amount: i64,
    expected: Option<i64>,
) -> Option<i64> {
    let mut client = mk_client(leader);
    trace!("getting leader last_id");
    let last_id = client.get_last_id();
    info!("executing leader transfer");
    let _sig = client
        .transfer(transfer_amount, &alice.keypair(), *bob_pubkey, &last_id)
        .unwrap();
    retry_get_balance(&mut client, bob_pubkey, expected)
}

fn retry_send_tx_and_retry_get_balance(
    leader: &NodeInfo,
    alice: &Mint,
    bob_pubkey: &Pubkey,
    expected: Option<i64>,
) -> Option<i64> {
    let mut client = mk_client(leader);
    trace!("getting leader last_id");
    let last_id = client.get_last_id();
    info!("executing leader transfer");
    const LAST: usize = 30;
    for run in 0..(LAST + 1) {
        let _sig = client
            .transfer(500, &alice.keypair(), *bob_pubkey, &last_id)
            .unwrap();
        let out = client.poll_get_balance(bob_pubkey);
        if expected.is_none() || run == LAST {
            return out.ok().clone();
        }
        trace!(
            "retry_send_tx_and_retry_get_balance[{}] {:?} {:?}",
            run,
            out,
            expected
        );
        if let (Some(e), Ok(o)) = (expected, out) {
            if o == e {
                return Some(o);
            }
        }
        sleep(Duration::from_millis(20));
    }
    None
}
