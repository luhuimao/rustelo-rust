extern crate bincode;
#[macro_use]
extern crate clap;
extern crate influx_db_client;
extern crate rayon;
extern crate serde_json;
#[macro_use]
extern crate buffett;

use clap::{App, Arg};
use influx_db_client as influxdb;
use rayon::prelude::*;
use buffett::client::mk_client;
use buffett::crdt::{Crdt, NodeInfo};
 use buffett::token_service::DRONE_PORT;
use buffett::hash::Hash;
use buffett::logger;
use buffett::metrics;
use buffett::ncp::Ncp;
use buffett::service::Service;
use buffett::signature::{read_keypair, GenKeys, Keypair, KeypairUtil};
use buffett::system_transaction::SystemTransaction;
use buffett::thin_client::{poll_gossip_for_leader, ThinClient};
use buffett::timing::{duration_as_ms, duration_as_s};
use buffett::transaction::Transaction;
use buffett::wallet::request_airdrop;
use buffett::window::default_window;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::process::exit;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::sleep;
use std::thread::Builder;
use std::time::Duration;
use std::time::Instant;
use buffett::asciiart; //mvp001
use std::io::Write; //mvp001

//mvp001
fn dividing_line() {
    println!("------------------------------------------------------------------------------------------------------------------------");
}
//*

pub struct NodeStats {
    pub tps: f64, // Maximum TPS reported by this node
    pub tx: u64,  // Total transactions reported by this node
}

fn metrics_submit_token_balance(token_balance: i64) {
    //println!("Token balance: {}", token_balance);
    metrics::submit(
        influxdb::Point::new("bench-tps")
            .add_tag("op", influxdb::Value::String("token_balance".to_string()))
            .add_field("balance", influxdb::Value::Integer(token_balance as i64))
            .to_owned(),
    );
}

fn sample_tx_count(
    exit_signal: &Arc<AtomicBool>,
    maxes: &Arc<RwLock<Vec<(SocketAddr, NodeStats)>>>,
    first_tx_count: u64,
    v: &NodeInfo,
    sample_period: u64,
) {
    let mut client = mk_client(&v);
    let mut now = Instant::now();
    let mut initial_tx_count = client.transaction_count();
    let mut max_tps = 0.0;
    let mut total;

    let log_prefix = format!("{:21}:", v.contact_info.tpu.to_string());

    loop {
        let tx_count = client.transaction_count();
        assert!(
            tx_count >= initial_tx_count,
            "expected tx_count({}) >= initial_tx_count({})",
            tx_count,
            initial_tx_count
        );
        let duration = now.elapsed();
        now = Instant::now();
        let sample = tx_count - initial_tx_count;
        initial_tx_count = tx_count;

        let ns = duration.as_secs() * 1_000_000_000 + u64::from(duration.subsec_nanos());
        let tps = (sample * 1_000_000_000) as f64 / ns as f64;
        if tps > max_tps {
            max_tps = tps;
        }
        if tx_count > first_tx_count {
            total = tx_count - first_tx_count;
        } else {
            total = 0;
        }
        
        /*mvp001
        println!(
            "{} {:9.2} TPS, Transactions: {:6}, Total transactions: {}",
            log_prefix, tps, sample, total
        );
        */
        let mut node_role = "Node's Role";
        //mvp002 check wether current client is leader or validator..
        if v.id == v.leader_id {
            //println!("This is Leader");
            node_role = "Leader   ";
        } else {
            // println!("This is Validator");
            node_role = "Validator";
        }
        let mut node_location = "Node Location";
        let mut node_ip: Vec<&str> = log_prefix.split(|c| c == '.' || c == ':').collect();
        //assert_eq!(node_ip, ["abc", "def", "ghi"]);
        if node_ip[0] == "192" && node_ip[1] == "168" {
            node_location = "LOCAL";
            //println!("{:?}", node_location);
        } else if node_ip[0] == "148"
            && node_ip[1] == "153"
            && node_ip[2] == "36"
            && node_ip[3] == "220"
        {
            node_location = "US_NEW_YORK";
            //println!("{:?}", node_location);
        } else if node_ip[0] == "148"
            && node_ip[1] == "153"
            && node_ip[2] == "50"
            && node_ip[3] == "162"
        {
            node_location = "DE_FRANKFURT";
            //println!("{:?}", node_location);
        } else if node_ip[0] == "148"
            && node_ip[1] == "153"
            && node_ip[2] == "25"
            && node_ip[3] == "50"
        {
            node_location = "NE_ARMSTERDAM";
            //println!("{:?}", node_location);
        } else if node_ip[0] == "164"
            && node_ip[1] == "52"
            && node_ip[2] == "39"
            && node_ip[3] == "162"
        {
            node_location = "SG_SINGAOPORE";
            //println!("{:?}", node_location);
        } else if node_ip[0] == "118"
            && node_ip[1] == "186"
            && node_ip[2] == "39"
            && node_ip[3] == "238"
        {
            node_location = "CN_PEKING";
            //println!("{:?}", node_location);
        }
        
        //mvp002
        println!(
            "| {0:13} {1:<8} {2:3}{3:20}|{4:>15}{5:>10.2} |{6:>15}{7:>13} |{8:19}{9:9}",
            node_location,
            node_role,
            "IP:",
            log_prefix,
            "Real-Time TPS:",
            tps,
            " Txs Proccessed:",
            sample,
            " Total Transactions:",
            total
        );


        sleep(Duration::new(sample_period, 0));

        if exit_signal.load(Ordering::Relaxed) {
            println!("\n| Exit Signal detected, kill threas for this Node:{}", log_prefix);
            print_animation_arrows();
            let stats = NodeStats {
                tps: max_tps,
                tx: total,
            };
            maxes.write().unwrap().push((v.contact_info.tpu, stats));
            break;
        }
    }
}

/// Send loopback payment of 0 tokens and confirm the network processed it
fn send_barrier_transaction(barrier_client: &mut ThinClient, last_id: &mut Hash, id: &Keypair) {
    let transfer_start = Instant::now();

    let mut poll_count = 0;
    loop {
        if poll_count > 0 && poll_count % 8 == 0 {
            
            /*
            println!(
                "polling for barrier transaction confirmation, attempt {}",
                poll_count
            );
            */
        }

        *last_id = barrier_client.get_last_id();
        let signature = barrier_client
            .transfer(0, &id, id.pubkey(), last_id)
            .expect("Unable to send barrier transaction");

        let confirmatiom = barrier_client.poll_for_signature(&signature);
        let duration_ms = duration_as_ms(&transfer_start.elapsed());
        if confirmatiom.is_ok() {
            //println!("barrier transaction confirmed in {}ms", duration_ms);

            metrics::submit(
                influxdb::Point::new("bench-tps")
                    .add_tag(
                        "op",
                        influxdb::Value::String("send_barrier_transaction".to_string()),
                    ).add_field("poll_count", influxdb::Value::Integer(poll_count))
                    .add_field("duration", influxdb::Value::Integer(duration_ms as i64))
                    .to_owned(),
            );

            // Sanity check that the client balance is still 1
            let balance = barrier_client
                .poll_balance_with_timeout(
                    &id.pubkey(),
                    &Duration::from_millis(100),
                    &Duration::from_secs(10),
                ).expect("Failed to get balance");
            if balance != 1 {
                panic!("Expected an account balance of 1 (balance: {}", balance);
            }
            break;
        }

        // Timeout after 3 minutes.  When running a CPU-only leader+validator+drone+bench-tps on a dev
        // machine, some batches of transactions can take upwards of 1 minute...
        if duration_ms > 1000 * 60 * 3 {
            println!("Error: Couldn't confirm barrier transaction!");
            exit(1);
        }

        let new_last_id = barrier_client.get_last_id();
        if new_last_id == *last_id {
            if poll_count > 0 && poll_count % 8 == 0 {
                println!("last_id is not advancing, still at {:?}", *last_id);
            }
        } else {
            *last_id = new_last_id;
        }

        poll_count += 1;
    }
}

fn generate_txs(
    shared_txs: &Arc<RwLock<VecDeque<Vec<Transaction>>>>,
    id: &Keypair,
    keypairs: &[Keypair],
    last_id: &Hash,
    threads: usize,
    reclaim: bool,
) {
    let tx_count = keypairs.len();
    //println!("Signing transactions... {} (reclaim={})", tx_count, reclaim);
    //mvp001
    dividing_line();
    println!(
        "{0: <2}{1: <40}: {2: <10}",
        "|", "Transactions to be signed", tx_count
    );
    println!(
        "{0: <2}{1: <40}: {2: <10}",
        "|", "Reclaimed Tokens", reclaim
    );
    dividing_line();
    //println!("Signing transactions... {} (reclaim={})", tx_count, reclaim);
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Status", "Signing Started"
    );
    dividing_line();
    //*

    let signing_start = Instant::now();

    let transactions: Vec<_> = keypairs
        .par_iter()
        .map(|keypair| {
            if !reclaim {
                Transaction::system_new(&id, keypair.pubkey(), 1, *last_id)
            } else {
                Transaction::system_new(keypair, id.pubkey(), 1, *last_id)
            }
        }).collect();

    let duration = signing_start.elapsed();
    let ns = duration.as_secs() * 1_000_000_000 + u64::from(duration.subsec_nanos());
    let bsps = (tx_count) as f64 / ns as f64;
    let nsps = ns as f64 / (tx_count) as f64;
    /*
    println!(
        "Done. {:.2} thousand signatures per second, {:.2} us per signature, {} ms total time",
        bsps * 1_000_000_f64,
        nsps / 1_000_f64,
        duration_as_ms(&duration),
    );
    */
    //mvp001
    dividing_line();
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Status", "Signing Finished"
    );
    println!(
        "{0: <2}Transaction Generated :{1:?} ,Time Consumed:{2:.2}, Speed:{3:?} in the last {4:.2 } milliseconds",
        "|",
        tx_count,
        ns/1_000_000_000_u64,
        bsps * 1_000_000_f64 * 1000_f64,
        duration_as_ms(&duration)
        
    );
    dividing_line();

    metrics::submit(
        influxdb::Point::new("bench-tps")
            .add_tag("op", influxdb::Value::String("generate_txs".to_string()))
            .add_field(
                "duration",
                influxdb::Value::Integer(duration_as_ms(&duration) as i64),
            ).to_owned(),
    );

    let sz = transactions.len() / threads;
    let chunks: Vec<_> = transactions.chunks(sz).collect();
    {
        let mut shared_txs_wl = shared_txs.write().unwrap();
        for chunk in chunks {
            shared_txs_wl.push_back(chunk.to_vec());
        }
    }
}

fn do_tx_transfers(
    exit_signal: &Arc<AtomicBool>,
    shared_txs: &Arc<RwLock<VecDeque<Vec<Transaction>>>>,
    leader: &NodeInfo,
    shared_tx_thread_count: &Arc<AtomicIsize>,
    total_tx_sent_count: &Arc<AtomicUsize>,
) {
    let client = mk_client(&leader);
    println!("| Begin to sendout transactions in parrallel");
    loop {
        let txs;
        {
            let mut shared_txs_wl = shared_txs.write().unwrap();
            txs = shared_txs_wl.pop_front();
        }
        if let Some(txs0) = txs {
            shared_tx_thread_count.fetch_add(1, Ordering::Relaxed);
            /*println!(
                "Transferring 1 unit {} times... to {}",
                txs0.len(),
                leader.contact_info.tpu
            );*/
            let tx_len = txs0.len();
            let transfer_start = Instant::now();
            for tx in txs0 {
                client.transfer_signed(&tx).unwrap();
            }
            shared_tx_thread_count.fetch_add(-1, Ordering::Relaxed);
            total_tx_sent_count.fetch_add(tx_len, Ordering::Relaxed);
            println!(
                //"Tx send done. {} ms {} tps",
                "| > 1 MU sent, to {} in {} ms, TPS: {} ",
                leader.contact_info.tpu,
                duration_as_ms(&transfer_start.elapsed()),
                tx_len as f32 / duration_as_s(&transfer_start.elapsed()),
            );
            metrics::submit(
                influxdb::Point::new("bench-tps")
                    .add_tag("op", influxdb::Value::String("do_tx_transfers".to_string()))
                    .add_field(
                        "duration",
                        influxdb::Value::Integer(duration_as_ms(&transfer_start.elapsed()) as i64),
                    ).add_field("count", influxdb::Value::Integer(tx_len as i64))
                    .to_owned(),
            );
        }
        if exit_signal.load(Ordering::Relaxed) {
            break;
        }
    }
}

fn airdrop_tokens(client: &mut ThinClient, leader: &NodeInfo, id: &Keypair, tx_count: i64) {
    let mut drone_addr = leader.contact_info.tpu;
    drone_addr.set_port(DRONE_PORT);

    let starting_balance = client.poll_get_balance(&id.pubkey()).unwrap_or(0);
    metrics_submit_token_balance(starting_balance);
    println!("starting balance {}", starting_balance);

    if starting_balance < tx_count {
        //mvp001
        println!("| Begin to prepare data and send some Transactions:",);
        dividing_line();
        print_animation_arrows();
        //*

        let airdrop_amount = tx_count - starting_balance;
        println!(
            "Airdropping {:?} tokens from {} for {}",
            airdrop_amount,
            drone_addr,
            id.pubkey(),
        );

        if let Err(e) = request_airdrop(&drone_addr, &id.pubkey(), airdrop_amount as u64) {
            panic!(
                "Error requesting airdrop: {:?} to addr: {:?} amount: {}",
                e, drone_addr, airdrop_amount
            );
        }

        // TODO: return airdrop Result from Drone instead of polling the
        //       network
        let mut current_balance = starting_balance;
        for _ in 0..20 {
            sleep(Duration::from_millis(500));
            current_balance = client.poll_get_balance(&id.pubkey()).unwrap_or_else(|e| {
                println!("airdrop error {}", e);
                starting_balance
            });
            if starting_balance != current_balance {
                break;
            }
            //println!("current balance {}...", current_balance);
            println!(
                "Current balance of {} is {}...",
                id.pubkey(),
                current_balance
            );
            //*
        }
        metrics_submit_token_balance(current_balance);
        if current_balance - starting_balance != airdrop_amount {
            println!(
                "Airdrop failed! {} {} {}",
                id.pubkey(),
                current_balance,
                starting_balance
            );
            exit(1);
        }
    }
}

fn compute_and_report_stats(
    maxes: &Arc<RwLock<Vec<(SocketAddr, NodeStats)>>>,
    sample_period: u64,
    tx_send_elapsed: &Duration,
    total_tx_send_count: usize,
) {
    // Compute/report stats
    let mut max_of_maxes = 0.0;
    let mut max_tx_count = 0;
    let mut nodes_with_zero_tps = 0;
    let mut total_maxes = 0.0;
    println!(" Node address        |       Max TPS | Total Transactions");
    println!("---------------------+---------------+--------------------");

    for (sock, stats) in maxes.read().unwrap().iter() {
        let maybe_flag = match stats.tx {
            0 => "!!!!!",
            _ => "",
        };

        println!(
            "{:20} | {:13.2} | {} {}",
            (*sock).to_string(),
            stats.tps,
            stats.tx,
            maybe_flag
        );

        if stats.tps == 0.0 {
            nodes_with_zero_tps += 1;
        }
        total_maxes += stats.tps;

        if stats.tps > max_of_maxes {
            max_of_maxes = stats.tps;
        }
        if stats.tx > max_tx_count {
            max_tx_count = stats.tx;
        }
    }

    if total_maxes > 0.0 {
        let num_nodes_with_tps = maxes.read().unwrap().len() - nodes_with_zero_tps;
        let average_max = total_maxes / num_nodes_with_tps as f64;
        println!("====================================================================================");
        println!("| Normal TPS:{:.2}",average_max);
        println!("====================================================================================");
        
        /*
        println!(
            "\nAverage max TPS: {:.2}, {} nodes had 0 TPS",
            average_max, nodes_with_zero_tps
        );
        */
    }

    println!("====================================================================================");
    println!("| Peak TPS:{:.2}",max_of_maxes);
    println!("====================================================================================");
    /*
    println!(
        "\nHighest TPS: {:.2} sampling period {}s max transactions: {} clients: {} drop rate: {:.2}",
        max_of_maxes,
        sample_period,
        max_tx_count,
        maxes.read().unwrap().len(),
        (total_tx_send_count as u64 - max_tx_count) as f64 / total_tx_send_count as f64,
    );*/

    println!(
        "\tAverage TPS: {}",
        max_tx_count as f32 / duration_as_s(tx_send_elapsed)
    );
}

// First transfer 3/4 of the tokens to the dest accounts
// then ping-pong 1/4 of the tokens back to the other account
// this leaves 1/4 token buffer in each account
fn should_switch_directions(num_tokens_per_account: i64, i: i64) -> bool {
    i % (num_tokens_per_account / 4) == 0 && (i >= (3 * num_tokens_per_account) / 4)
}

//mvp001
// mvp006
fn print_animation_arrows(){
    print!("|\n|");
    for _ in 0..5 {
        print!(".");
        sleep(Duration::from_millis(300));
        std::io::stdout().flush().ok().expect("some error message");
    }
    print!("\n|\n");
    
}
// Node selections process
// Showcase the leader node selection process
fn leader_node_selection(){
    dividing_line();
    println!("| {}","Selecting Transaction Validator Nodes from the Predefined High-Reputation Nodes List.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    println!("| {}","HRNL is populated with hundreds, even thousands of candidate nodes.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    println!("| {}","An random process is evoked to select up to 21 nodes from this list.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    println!("| {}","These 21 nodes are responsible for validating transactions on the DLT network.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    println!("| {}","They are further grouped into one leader node and 20 voting nodes.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    println!("| {}","For MVP demo, we only use 5 nodes from 5 different countries.");
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");
    dividing_line();
    sleep(Duration::from_millis(100));
    std::io::stdout().flush().ok().expect("some error message");

    print_animation_arrows();

    
    let mut json_input="/path/path/path.ext";
    let mut leader_node = randome_select_from(json_input);
    dividing_line();
    println!("| {}","Transaction Validator Nodes Selection Process Complete!!");
    dividing_line();
    
}
// mvp001
fn randome_select_from(path: &str)-> String{
    //do some selection 
    let mut node = String::new();
    node = "node1_addr_in_pubkey_form".to_string();
    node 
}
//*

fn main() {
    logger::setup();
    metrics::set_panic_hook("bench-tps");

    let matches = App::new("bitconch-bench-tps")
        .version(crate_version!())
        .arg(
            Arg::with_name("network")
                .short("n")
                .long("network")
                .value_name("HOST:PORT")
                .takes_value(true)
                .help("Rendezvous with the network at this gossip entry point; defaults to 127.0.0.1:8001"),
        )
        .arg(
            Arg::with_name("identity")
                .short("i")
                .long("identity")
                .value_name("PATH")
                .takes_value(true)
                .required(true)
                .help("File containing a client identity (keypair)"),
        )
        .arg(
            Arg::with_name("num-nodes")
                .short("N")
                .long("num-nodes")
                .value_name("NUM")
                .takes_value(true)
                .help("Wait for NUM nodes to converge"),
        )
        .arg(
            Arg::with_name("reject-extra-nodes")
                .long("reject-extra-nodes")
                .help("Require exactly `num-nodes` on convergence. Appropriate only for internal networks"),
        )
        .arg(
            Arg::with_name("threads")
                .short("t")
                .long("threads")
                .value_name("NUM")
                .takes_value(true)
                .help("Number of threads"),
        )
        .arg(
            Arg::with_name("duration")
                .long("duration")
                .value_name("SECS")
                .takes_value(true)
                .help("Seconds to run benchmark, then exit; default is forever"),
        )
        .arg(
            Arg::with_name("converge-only")
                .long("converge-only")
                .help("Exit immediately after converging"),
        )
        .arg(
            Arg::with_name("sustained")
                .long("sustained")
                .help("Use sustained performance mode vs. peak mode. This overlaps the tx generation with transfers."),
        )
        .arg(
            Arg::with_name("tx_count")
                .long("tx_count")
                .value_name("NUM")
                .takes_value(true)
                .help("Number of transactions to send per batch")
        )
        .get_matches();

    let network = if let Some(addr) = matches.value_of("network") {
        addr.parse().unwrap_or_else(|e| {
            eprintln!("failed to parse network: {}", e);
            exit(1)
        })
    } else {
        socketaddr!("127.0.0.1:8001")
    };

    let id =
        read_keypair(matches.value_of("identity").unwrap()).expect("can't read client identity");

    let threads = if let Some(t) = matches.value_of("threads") {
        t.to_string().parse().expect("can't parse threads")
    } else {
        4usize
    };

    let num_nodes = if let Some(n) = matches.value_of("num-nodes") {
        n.to_string().parse().expect("can't parse num-nodes")
    } else {
        1usize
    };

    let duration = if let Some(s) = matches.value_of("duration") {
        Duration::new(s.to_string().parse().expect("can't parse duration"), 0)
    } else {
        Duration::new(std::u64::MAX, 0)
    };

    let tx_count = if let Some(s) = matches.value_of("tx_count") {
        s.to_string().parse().expect("can't parse tx_count")
    } else {
        500_000
    };

    let sustained = matches.is_present("sustained");

    asciiart::welcome();//mvp001
    dividing_line();//mvp001
    leader_node_selection();//mvp001

    //println!("Looking for leader at {:?}", network);
    //mvp001
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Search for Leader Node On Network", network
    );
    dividing_line();
    print_animation_arrows();
    //*

    let leader = poll_gossip_for_leader(network, None).expect("unable to find leader on network");

    let exit_signal = Arc::new(AtomicBool::new(false));
    //mvp001
    dividing_line();
    println!(
        "| Leader Node is found!, ID: {:?}",
        &leader.id
    );
    dividing_line();
    sleep(Duration::from_millis(100));
    //*
    let (nodes, leader, ncp) = converge(&leader, &exit_signal, num_nodes);

    if nodes.len() < num_nodes {
        println!(
            "Error: Insufficient nodes discovered.  Expecting {} or more",
            num_nodes
        );
        exit(1);
    }
    if matches.is_present("reject-extra-nodes") && nodes.len() > num_nodes {
        println!(
            "Error: Extra nodes discovered.  Expecting exactly {}",
            num_nodes
        );
        exit(1);
    }

    if leader.is_none() {
        println!("no leader");
        exit(1);
    }

    if matches.is_present("converge-only") {
        return;
    }

    let leader = leader.unwrap();

    //mvp001
    dividing_line();
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Leader Node Contact Information", leader.contact_info.rpu
    );
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Leader Node ID", leader.id
    );
    dividing_line();
    //*
    //println!("leader is at {} {}", leader.contact_info.rpu, leader.id);
    
    let mut client = mk_client(&leader);
    let mut barrier_client = mk_client(&leader);

    let mut seed = [0u8; 32];
    seed.copy_from_slice(&id.public_key_bytes()[..32]);
    let mut rnd = GenKeys::new(seed);

    //mvp
    println!("| Begin to prepare data and send some Transactions:");
    dividing_line();
    print_animation_arrows();
    //println!("Creating {} keypairs...", tx_count / 2);
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|",
        "Create Key Pairs",
        tx_count / 2
    );
    //*

    let keypairs = rnd.gen_n_keypairs(tx_count / 2);
    let barrier_id = rnd.gen_n_keypairs(1).pop().unwrap();

    //mvp001
    print_animation_arrows();
    println!(
        "{0: <2}{1: <40}: {2: <60}",
        "|", "Issue Tokens", "Yes, issue some tokens to each account."
    );
    //*
    //println!("Get tokens...");
    let num_tokens_per_account = 20;

    // Sample the first keypair, see if it has tokens, if so then resume
    // to avoid token loss
    let keypair0_balance = client.poll_get_balance(&keypairs[0].pubkey()).unwrap_or(0);

    if num_tokens_per_account > keypair0_balance {
        airdrop_tokens(
            &mut client,
            &leader,
            &id,
            (num_tokens_per_account - keypair0_balance) * tx_count,
        );
    }
    airdrop_tokens(&mut barrier_client, &leader, &barrier_id, 1);

    //println!("Get last ID...");
    let mut last_id = client.get_last_id();
    //println!("Got last ID {:?}", last_id);

    let first_tx_count = client.transaction_count();
    println!("Initial transaction count {}", first_tx_count);

    // Setup a thread per validator to sample every period
    // collect the max transaction rate and total tx count seen
    let maxes = Arc::new(RwLock::new(Vec::new()));
    let sample_period = 1; // in seconds
    println!("Sampling TPS every {} second...", sample_period);
    let v_threads: Vec<_> = nodes
        .into_iter()
        .map(|v| {
            let exit_signal = exit_signal.clone();
            let maxes = maxes.clone();
            Builder::new()
                .name("bitconch-client-sample".to_string())
                .spawn(move || {
                    sample_tx_count(&exit_signal, &maxes, first_tx_count, &v, sample_period);
                }).unwrap()
        }).collect();

    let shared_txs: Arc<RwLock<VecDeque<Vec<Transaction>>>> =
        Arc::new(RwLock::new(VecDeque::new()));

    let shared_tx_active_thread_count = Arc::new(AtomicIsize::new(0));
    let total_tx_sent_count = Arc::new(AtomicUsize::new(0));

    let s_threads: Vec<_> = (0..threads)
        .map(|_| {
            let exit_signal = exit_signal.clone();
            let shared_txs = shared_txs.clone();
            let leader = leader.clone();
            let shared_tx_active_thread_count = shared_tx_active_thread_count.clone();
            let total_tx_sent_count = total_tx_sent_count.clone();
            Builder::new()
                .name("bitconch-client-sender".to_string())
                .spawn(move || {
                    do_tx_transfers(
                        &exit_signal,
                        &shared_txs,
                        &leader,
                        &shared_tx_active_thread_count,
                        &total_tx_sent_count,
                    );
                }).unwrap()
        }).collect();

    // generate and send transactions for the specified duration
    let start = Instant::now();
    let mut reclaim_tokens_back_to_source_account = false;
    let mut i = keypair0_balance;
    while start.elapsed() < duration {
        let balance = client.poll_get_balance(&id.pubkey()).unwrap_or(-1);
        metrics_submit_token_balance(balance);

        // ping-pong between source and destination accounts for each loop iteration
        // this seems to be faster than trying to determine the balance of individual
        // accounts
        generate_txs(
            &shared_txs,
            &id,
            &keypairs,
            &last_id,
            threads,
            reclaim_tokens_back_to_source_account,
        );
        // In sustained mode overlap the transfers with generation
        // this has higher average performance but lower peak performance
        // in tested environments.
        if !sustained {
            while shared_tx_active_thread_count.load(Ordering::Relaxed) > 0 {
                sleep(Duration::from_millis(100));
            }
        }
        // It's not feasible (would take too much time) to confirm each of the `tx_count / 2`
        // transactions sent by `generate_txs()` so instead send and confirm a single transaction
        // to validate the network is still functional.
        send_barrier_transaction(&mut barrier_client, &mut last_id, &barrier_id);

        i += 1;
        if should_switch_directions(num_tokens_per_account, i) {
            reclaim_tokens_back_to_source_account = !reclaim_tokens_back_to_source_account;
        }
    }

    // Stop the sampling threads so it will collect the stats
    exit_signal.store(true, Ordering::Relaxed);

    dividing_line(); //mvp001
    println!("| Kill all the remaining threads.");
    print_animation_arrows();
    for t in v_threads {
        if let Err(err) = t.join() {
            println!("  join() failed with: {:?}", err);
        }
    }

    // join the tx send threads
    //println!("Waiting for transmit threads...");
    for t in s_threads {
        if let Err(err) = t.join() {
            println!("  join() failed with: {:?}", err);
        }
    }

    let balance = client.poll_get_balance(&id.pubkey()).unwrap_or(-1);
    metrics_submit_token_balance(balance);

    compute_and_report_stats(
        &maxes,
        sample_period,
        &start.elapsed(),
        total_tx_sent_count.load(Ordering::Relaxed),
    );

    // join the crdt client threads
    ncp.join().unwrap();
}

fn converge(
    leader: &NodeInfo,
    exit_signal: &Arc<AtomicBool>,
    num_nodes: usize,
) -> (Vec<NodeInfo>, Option<NodeInfo>, Ncp) {
    //lets spy on the network
    let (node, gossip_socket) = Crdt::spy_node();
    let mut spy_crdt = Crdt::new(node).expect("Crdt::new");
    spy_crdt.insert(&leader);
    spy_crdt.set_leader(leader.id);
    let spy_ref = Arc::new(RwLock::new(spy_crdt));
    let window = Arc::new(RwLock::new(default_window()));
    let ncp = Ncp::new(&spy_ref, window, None, gossip_socket, exit_signal.clone());
    let mut v: Vec<NodeInfo> = vec![];
    // wait for the network to converge, 30 seconds should be plenty
    for _ in 0..30 {
        {
            let spy_ref = spy_ref.read().unwrap();

            println!("{}", spy_ref.node_info_trace());

            if spy_ref.leader_data().is_some() {
                v = spy_ref
                    .table
                    .values()
                    .filter(|x| Crdt::is_valid_address(&x.contact_info.rpu))
                    .cloned()
                    .collect();

                if v.len() >= num_nodes {
                    println!("CONVERGED!");
                    break;
                } else {
                    println!(
                        "{} node(s) discovered (looking for {} or more)",
                        v.len(),
                        num_nodes
                    );
                }
            }
        }
        sleep(Duration::new(1, 0));
    }
    let leader = spy_ref.read().unwrap().leader_data().cloned();
    (v, leader, ncp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_switch_directions() {
        assert_eq!(should_switch_directions(20, 0), false);
        assert_eq!(should_switch_directions(20, 1), false);
        assert_eq!(should_switch_directions(20, 14), false);
        assert_eq!(should_switch_directions(20, 15), true);
        assert_eq!(should_switch_directions(20, 16), false);
        assert_eq!(should_switch_directions(20, 19), false);
        assert_eq!(should_switch_directions(20, 20), true);
        assert_eq!(should_switch_directions(20, 21), false);
        assert_eq!(should_switch_directions(20, 99), false);
        assert_eq!(should_switch_directions(20, 100), true);
        assert_eq!(should_switch_directions(20, 101), false);
    }
}
