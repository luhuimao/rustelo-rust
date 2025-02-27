use crate::bank::Bank;
use soros_sdk::client::{AsyncClient, Client, SyncClient};
use soros_sdk::hash::Hash;
use soros_sdk::instruction::Instruction;
use soros_sdk::message::Message;
use soros_sdk::pubkey::Pubkey;
use soros_sdk::signature::Signature;
use soros_sdk::signature::{Keypair, KeypairUtil};
use soros_sdk::system_instruction;
use soros_sdk::transaction::{self, Transaction};
use soros_sdk::transport::{Result, TransportError};
use std::io;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::{sleep, Builder};
use std::time::{Duration, Instant};

pub struct BankClient {
    bank: Arc<Bank>,
    transaction_sender: Mutex<Sender<Transaction>>,
}

impl Client for BankClient {
    fn transactions_addr(&self) -> String {
        "Local BankClient".to_string()
    }
}

impl AsyncClient for BankClient {
    fn async_send_transaction(&self, transaction: Transaction) -> io::Result<Signature> {
        let signature = transaction.signatures.get(0).cloned().unwrap_or_default();
        let transaction_sender = self.transaction_sender.lock().unwrap();
        transaction_sender.send(transaction).unwrap();
        Ok(signature)
    }

    fn async_send_message(
        &self,
        keypairs: &[&Keypair],
        message: Message,
        recent_blockhash: Hash,
    ) -> io::Result<Signature> {
        let transaction = Transaction::new(&keypairs, message, recent_blockhash);
        self.async_send_transaction(transaction)
    }

    fn async_send_instruction(
        &self,
        keypair: &Keypair,
        instruction: Instruction,
        recent_blockhash: Hash,
    ) -> io::Result<Signature> {
        let message = Message::new(vec![instruction]);
        self.async_send_message(&[keypair], message, recent_blockhash)
    }

    /// Transfer `dif` from `keypair` to `pubkey`
    fn async_transfer(
        &self,
        // lamports: u64,
        dif: u64,
        keypair: &Keypair,
        pubkey: &Pubkey,
        recent_blockhash: Hash,
    ) -> io::Result<Signature> {
        let transfer_instruction =
            // system_instruction::transfer(&keypair.pubkey(), pubkey, lamports);
            system_instruction::transfer(&keypair.pubkey(), pubkey, dif);
        self.async_send_instruction(keypair, transfer_instruction, recent_blockhash)
    }
}

impl SyncClient for BankClient {
    fn send_message(&self, keypairs: &[&Keypair], message: Message) -> Result<Signature> {
        let blockhash = self.bank.last_blockhash();
        let transaction = Transaction::new(&keypairs, message, blockhash);
        self.bank.process_transaction(&transaction)?;
        Ok(transaction.signatures.get(0).cloned().unwrap_or_default())
    }

    /// Create and process a transaction from a single instruction.
    fn send_instruction(&self, keypair: &Keypair, instruction: Instruction) -> Result<Signature> {
        let message = Message::new(vec![instruction]);
        self.send_message(&[keypair], message)
    }

    /// Transfer `dif` from `keypair` to `pubkey`
    // fn transfer(&self, lamports: u64, keypair: &Keypair, pubkey: &Pubkey) -> Result<Signature> {
    fn transfer(&self, dif: u64, keypair: &Keypair, pubkey: &Pubkey) -> Result<Signature> {
        let transfer_instruction =
            // system_instruction::transfer(&keypair.pubkey(), pubkey, lamports);
            system_instruction::transfer(&keypair.pubkey(), pubkey, dif);
        self.send_instruction(keypair, transfer_instruction)
    }

    fn get_account_data(&self, pubkey: &Pubkey) -> Result<Option<Vec<u8>>> {
        Ok(self.bank.get_account(pubkey).map(|account| account.data))
    }

    fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        Ok(self.bank.get_balance(pubkey))
    }

    fn get_signature_status(
        &self,
        signature: &Signature,
    ) -> Result<Option<transaction::Result<()>>> {
        Ok(self.bank.get_signature_status(signature))
    }

    fn get_recent_blockhash(&self) -> Result<Hash> {
        let last_blockhash = self.bank.last_blockhash();
        Ok(last_blockhash)
    }

    fn get_transaction_count(&self) -> Result<u64> {
        Ok(self.bank.transaction_count())
    }

    fn poll_for_signature_confirmation(
        &self,
        signature: &Signature,
        min_confirmed_blocks: usize,
    ) -> Result<()> {
        let mut now = Instant::now();
        let mut confirmed_blocks = 0;
        loop {
            let response = self.bank.get_signature_confirmation_status(signature);
            if let Some((confirmations, res)) = response {
                if res.is_ok() {
                    if confirmed_blocks != confirmations {
                        now = Instant::now();
                        confirmed_blocks = confirmations;
                    }
                    if confirmations >= min_confirmed_blocks {
                        break;
                    }
                }
            };
            if now.elapsed().as_secs() > 15 {
                // TODO: Return a better error.
                return Err(TransportError::IoError(io::Error::new(
                    io::ErrorKind::Other,
                    "signature not found",
                )));
            }
            sleep(Duration::from_millis(250));
        }
        Ok(())
    }

    fn poll_for_signature(&self, signature: &Signature) -> Result<()> {
        let now = Instant::now();
        loop {
            let response = self.bank.get_signature_status(signature);
            if let Some(res) = response {
                if res.is_ok() {
                    break;
                }
            }
            if now.elapsed().as_secs() > 15 {
                // TODO: Return a better error.
                return Err(TransportError::IoError(io::Error::new(
                    io::ErrorKind::Other,
                    "signature not found",
                )));
            }
            sleep(Duration::from_millis(250));
        }
        Ok(())
    }
}

impl BankClient {
    fn run(bank: &Bank, transaction_receiver: Receiver<Transaction>) {
        while let Ok(tx) = transaction_receiver.recv() {
            let mut transactions = vec![tx];
            while let Ok(tx) = transaction_receiver.try_recv() {
                transactions.push(tx);
            }
            let _ = bank.process_transactions(&transactions);
        }
    }

    pub fn new(bank: Bank) -> Self {
        let bank = Arc::new(bank);
        let (transaction_sender, transaction_receiver) = channel();
        let transaction_sender = Mutex::new(transaction_sender);
        let thread_bank = bank.clone();
        let bank = bank.clone();
        Builder::new()
            .name("soros-bank-client".to_string())
            .spawn(move || Self::run(&thread_bank, transaction_receiver))
            .unwrap();
        Self {
            bank,
            transaction_sender,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soros_sdk::genesis_block::GenesisBlock;
    use soros_sdk::instruction::AccountMeta;

    #[test]
    fn test_bank_client_new_with_keypairs() {
        let (genesis_block, john_doe_keypair) = GenesisBlock::new(10_000);
        let john_pubkey = john_doe_keypair.pubkey();
        let jane_doe_keypair = Keypair::new();
        let jane_pubkey = jane_doe_keypair.pubkey();
        let doe_keypairs = vec![&john_doe_keypair, &jane_doe_keypair];
        let bank = Bank::new(&genesis_block);
        let bank_client = BankClient::new(bank);

        // Create 2-2 Multisig Transfer instruction.
        let bob_pubkey = Pubkey::new_rand();
        let mut transfer_instruction = system_instruction::transfer(&john_pubkey, &bob_pubkey, 42);
        transfer_instruction
            .accounts
            .push(AccountMeta::new(jane_pubkey, true));

        let message = Message::new(vec![transfer_instruction]);
        bank_client.send_message(&doe_keypairs, message).unwrap();
        assert_eq!(bank_client.get_balance(&bob_pubkey).unwrap(), 42);
    }
}
