use crate::budget_expr::BudgetExpr;
use crate::budget_state::BudgetState;
use crate::id;
use bincode::serialized_size;
use chrono::prelude::{DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use soros_sdk::instruction::{AccountMeta, Instruction};
use soros_sdk::pubkey::Pubkey;
use soros_sdk::system_instruction;

/// A smart contract.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct Contract {
    /// The number of dif allocated to the `BudgetExpr` and any transaction fees.
    // pub lamports: u64,
    pub dif: u64,
    pub budget_expr: BudgetExpr,
}

/// An instruction to progress the smart contract.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub enum BudgetInstruction {
    /// Declare and instantiate `BudgetExpr`.
    InitializeAccount(BudgetExpr),

    /// Tell a payment plan acknowledge the given `DateTime` has past.
    ApplyTimestamp(DateTime<Utc>),

    /// Tell the budget that the `InitializeAccount` with `Signature` has been
    /// signed by the containing transaction's `Pubkey`.
    ApplySignature,
}

fn initialize_account(contract: &Pubkey, expr: BudgetExpr) -> Instruction {
    let mut keys = vec![];
    if let BudgetExpr::Pay(payment) = &expr {
        keys.push(AccountMeta::new(payment.to, false));
    }
    keys.push(AccountMeta::new(*contract, false));
    Instruction::new(id(), &BudgetInstruction::InitializeAccount(expr), keys)
}

pub fn create_account(
    from: &Pubkey,
    contract: &Pubkey,
    // lamports: u64,
    dif: u64,
    expr: BudgetExpr,
) -> Vec<Instruction> {
    // if !expr.verify(lamports) {
    if !expr.verify(dif) {
        panic!("invalid budget expression");
    }
    let space = serialized_size(&BudgetState::new(expr.clone())).unwrap();
    vec![
        // system_instruction::create_account(&from, contract, lamports, space, &id()),
        system_instruction::create_account(&from, contract, dif, space, &id()),
        initialize_account(contract, expr),
    ]
}

/// Create a new payment script.
// pub fn payment(from: &Pubkey, to: &Pubkey, lamports: u64) -> Vec<Instruction> {
pub fn payment(from: &Pubkey, to: &Pubkey, dif: u64) -> Vec<Instruction> {
    let contract = Pubkey::new_rand();
    // let expr = BudgetExpr::new_payment(lamports, to);
    let expr = BudgetExpr::new_payment(dif, to);
    // create_account(from, &contract, lamports, expr)
    create_account(from, &contract, dif, expr)
}

/// Create a future payment script.
pub fn on_date(
    from: &Pubkey,
    to: &Pubkey,
    contract: &Pubkey,
    dt: DateTime<Utc>,
    dt_pubkey: &Pubkey,
    cancelable: Option<Pubkey>,
    // lamports: u64,
    dif: u64,
) -> Vec<Instruction> {
    // let expr = BudgetExpr::new_cancelable_future_payment(dt, dt_pubkey, lamports, to, cancelable);
    let expr = BudgetExpr::new_cancelable_future_payment(dt, dt_pubkey, dif, to, cancelable);
    // create_account(from, contract, lamports, expr)
    create_account(from, contract, dif, expr)
}

/// Create a multisig payment script.
pub fn when_signed(
    from: &Pubkey,
    to: &Pubkey,
    contract: &Pubkey,
    witness: &Pubkey,
    cancelable: Option<Pubkey>,
    // lamports: u64,
    dif: u64,
) -> Vec<Instruction> {
    // let expr = BudgetExpr::new_cancelable_authorized_payment(witness, lamports, to, cancelable);
    let expr = BudgetExpr::new_cancelable_authorized_payment(witness, dif, to, cancelable);
    // create_account(from, contract, lamports, expr)
    create_account(from, contract, dif, expr)
}

pub fn apply_timestamp(
    from: &Pubkey,
    contract: &Pubkey,
    to: &Pubkey,
    dt: DateTime<Utc>,
) -> Instruction {
    let mut account_metas = vec![
        AccountMeta::new(*from, true),
        AccountMeta::new(*contract, false),
    ];
    if from != to {
        account_metas.push(AccountMeta::new(*to, false));
    }
    Instruction::new(id(), &BudgetInstruction::ApplyTimestamp(dt), account_metas)
}

pub fn apply_signature(from: &Pubkey, contract: &Pubkey, to: &Pubkey) -> Instruction {
    let mut account_metas = vec![
        AccountMeta::new(*from, true),
        AccountMeta::new(*contract, false),
    ];
    if from != to {
        account_metas.push(AccountMeta::new(*to, false));
    }
    Instruction::new(id(), &BudgetInstruction::ApplySignature, account_metas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget_expr::BudgetExpr;

    #[test]
    fn test_budget_instruction_verify() {
        let alice_pubkey = Pubkey::new_rand();
        let bob_pubkey = Pubkey::new_rand();
        payment(&alice_pubkey, &bob_pubkey, 1); // No panic! indicates success.
    }

    #[test]
    #[should_panic]
    fn test_budget_instruction_overspend() {
        let alice_pubkey = Pubkey::new_rand();
        let bob_pubkey = Pubkey::new_rand();
        let budget_pubkey = Pubkey::new_rand();
        let expr = BudgetExpr::new_payment(2, &bob_pubkey);
        create_account(&alice_pubkey, &budget_pubkey, 1, expr);
    }

    #[test]
    #[should_panic]
    fn test_budget_instruction_underspend() {
        let alice_pubkey = Pubkey::new_rand();
        let bob_pubkey = Pubkey::new_rand();
        let budget_pubkey = Pubkey::new_rand();
        let expr = BudgetExpr::new_payment(1, &bob_pubkey);
        create_account(&alice_pubkey, &budget_pubkey, 2, expr);
    }
}
