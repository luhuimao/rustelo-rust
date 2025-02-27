use bincode::serialize;
use log::*;
use reqwest;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};
use soros::fullnode::new_fullnode_for_tests;
use soros_client::rpc_client::get_rpc_request_str;
use soros_sdk::hash::Hash;
use soros_sdk::pubkey::Pubkey;
use soros_sdk::system_transaction;
use std::fs::remove_dir_all;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn test_rpc_send_tx() {
    soros_logger::setup();

    let (server, leader_data, alice, ledger_path) = new_fullnode_for_tests();
    let bob_pubkey = Pubkey::new_rand();

    let client = reqwest::Client::new();
    let request = json!({
       "jsonrpc": "2.0",
       "id": 1,
       "method": "getLatestBlockhash",
       "params": json!([])
    });
    let rpc_addr = leader_data.rpc;
    let rpc_string = get_rpc_request_str(rpc_addr, false);
    let mut response = client
        .post(&rpc_string)
        .header(CONTENT_TYPE, "application/json")
        .body(request.to_string())
        .send()
        .unwrap();
    let json: Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    let blockhash_vec = bs58::decode(json["result"].as_str().unwrap())
        .into_vec()
        .unwrap();
    let blockhash = Hash::new(&blockhash_vec);

    info!("blockhash: {:?}", blockhash);
    let tx = system_transaction::transfer(&alice, &bob_pubkey, 20, blockhash, 0);
    let serial_tx = serialize(&tx).unwrap();

    let client = reqwest::Client::new();
    let request = json!({
       "jsonrpc": "2.0",
       "id": 1,
       "method": "sendTxn",
       "params": json!([serial_tx])
    });
    let rpc_addr = leader_data.rpc;
    let rpc_string = get_rpc_request_str(rpc_addr, false);
    let mut response = client
        .post(&rpc_string)
        .header(CONTENT_TYPE, "application/json")
        .body(request.to_string())
        .send()
        .unwrap();
    let json: Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    let signature = &json["result"];

    let mut confirmed_tx = false;

    let client = reqwest::Client::new();
    let request = json!({
       "jsonrpc": "2.0",
       "id": 1,
       "method": "confirmTxn",
       "params": [signature],
    });

    for _ in 0..soros_sdk::timing::DEFAULT_TICKS_PER_SLOT {
        let mut response = client
            .post(&rpc_string)
            .header(CONTENT_TYPE, "application/json")
            .body(request.to_string())
            .send()
            .unwrap();
        let response_json_text = response.text().unwrap();
        let json: Value = serde_json::from_str(&response_json_text).unwrap();

        if true == json["result"] {
            confirmed_tx = true;
            break;
        }

        sleep(Duration::from_millis(500));
    }

    assert_eq!(confirmed_tx, true);

    server.close().unwrap();
    remove_dir_all(ledger_path).unwrap();
}
