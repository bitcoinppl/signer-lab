use std::collections::HashSet;

use bdk_wallet::bitcoin::{Amount, FeeRate, OutPoint, Psbt, ScriptBuf};
use cove_bdk::coin_selection::CoveDefaultCoinSelection;
use cove_util::ResultExt as _;
use serde::{Deserialize, Serialize};

use crate::{LabFeeRate, OperationKind, WalletAccount};

const MAX_FAN_OUT_OUTPUTS: usize = 10_000;
const MAX_BATCH_TRANSACTIONS: usize = 1_000;

/// A checked, non-empty set of destination scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationScripts(Vec<ScriptBuf>);

impl DestinationScripts {
    /// Creates a destination set for fan-out or independent batch planning.
    ///
    /// # Errors
    ///
    /// Returns an error when the list is empty or exceeds 10,000 scripts.
    pub fn new(scripts: Vec<ScriptBuf>) -> Result<Self, BulkPlanError> {
        if scripts.is_empty() {
            return Err(BulkPlanError::NoDestinations);
        }

        if scripts.len() > MAX_FAN_OUT_OUTPUTS {
            return Err(BulkPlanError::TooManyDestinations(scripts.len()));
        }

        Ok(Self(scripts))
    }

    /// Returns the number of destination scripts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true only when no scripts are present.
    ///
    /// Checked instances are never empty. This method supports generic collection views.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A checked bulk transaction request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkRequest(BulkRequestKind);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BulkRequestKind {
    FanOut {
        amount_per_output: Amount,
        destinations: DestinationScripts,
    },
    Batch {
        amount_per_transaction: Amount,
        destinations: DestinationScripts,
    },
    Consolidate {
        destination: ScriptBuf,
        selected_inputs: Option<Vec<OutPoint>>,
    },
}

impl BulkRequest {
    /// Creates one transaction with one output for every destination.
    ///
    /// # Errors
    ///
    /// Returns an error when the amount is zero.
    pub fn fan_out(
        amount_per_output: Amount,
        destinations: DestinationScripts,
    ) -> Result<Self, BulkPlanError> {
        require_positive_amount(amount_per_output)?;

        Ok(Self(BulkRequestKind::FanOut {
            amount_per_output,
            destinations,
        }))
    }

    /// Creates one independent transaction for every destination.
    ///
    /// # Errors
    ///
    /// Returns an error when the amount is zero or more than 1,000 transactions are requested.
    pub fn batch(
        amount_per_transaction: Amount,
        destinations: DestinationScripts,
    ) -> Result<Self, BulkPlanError> {
        require_positive_amount(amount_per_transaction)?;
        if destinations.len() > MAX_BATCH_TRANSACTIONS {
            return Err(BulkPlanError::TooManyBatchTransactions(destinations.len()));
        }

        Ok(Self(BulkRequestKind::Batch {
            amount_per_transaction,
            destinations,
        }))
    }

    /// Creates a transaction that consolidates every available UTXO.
    #[must_use]
    pub fn consolidate_all(destination: ScriptBuf) -> Self {
        Self(BulkRequestKind::Consolidate {
            destination,
            selected_inputs: None,
        })
    }

    /// Creates a transaction that consolidates only selected UTXOs.
    ///
    /// # Errors
    ///
    /// Returns an error when no outpoints are selected or an outpoint is repeated.
    pub fn consolidate_selected(
        destination: ScriptBuf,
        selected_inputs: Vec<OutPoint>,
    ) -> Result<Self, BulkPlanError> {
        if selected_inputs.is_empty() {
            return Err(BulkPlanError::NoSelectedInputs);
        }

        let unique = selected_inputs.iter().copied().collect::<HashSet<_>>();
        if unique.len() != selected_inputs.len() {
            return Err(BulkPlanError::DuplicateSelectedInput);
        }

        Ok(Self(BulkRequestKind::Consolidate {
            destination,
            selected_inputs: Some(selected_inputs),
        }))
    }

    /// Returns the operation behavior.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        match self.0 {
            BulkRequestKind::FanOut { .. } => OperationKind::FanOut,
            BulkRequestKind::Batch { .. } => OperationKind::Batch,
            BulkRequestKind::Consolidate { .. } => OperationKind::Consolidate,
        }
    }
}

/// One planned PSBT and its review values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedTransaction {
    #[serde(with = "psbt_serde")]
    psbt: Psbt,
    fee: Amount,
    actual_fee_rate: FeeRate,
    sent_amount: Amount,
}

impl PlannedTransaction {
    /// Returns the unsigned PSBT.
    #[must_use]
    pub const fn psbt(&self) -> &Psbt {
        &self.psbt
    }

    /// Returns a mutable PSBT for signing.
    #[must_use]
    pub const fn psbt_mut(&mut self) -> &mut Psbt {
        &mut self.psbt
    }

    /// Returns the calculated miner fee.
    #[must_use]
    pub const fn fee(&self) -> Amount {
        self.fee
    }

    /// Returns the actual fee rate after integer weight rounding.
    #[must_use]
    pub const fn actual_fee_rate(&self) -> FeeRate {
        self.actual_fee_rate
    }

    /// Returns the amount assigned to non-change destinations.
    #[must_use]
    pub const fn sent_amount(&self) -> Amount {
        self.sent_amount
    }

    /// Returns the transaction input count.
    #[must_use]
    pub fn input_count(&self) -> usize {
        self.psbt.unsigned_tx.input.len()
    }

    /// Returns the transaction output count, including change.
    #[must_use]
    pub fn output_count(&self) -> usize {
        self.psbt.unsigned_tx.output.len()
    }

    fn input_outpoints(&self) -> impl Iterator<Item = OutPoint> + '_ {
        self.psbt
            .unsigned_tx
            .input
            .iter()
            .map(|input| input.previous_output)
    }
}

/// A recoverable bulk plan whose inputs are reserved in the BDK wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkPlan {
    kind: OperationKind,
    requested_fee_rate: LabFeeRate,
    transactions: Vec<PlannedTransaction>,
}

impl BulkPlan {
    /// Returns the operation behavior.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        self.kind
    }

    /// Returns the requested fee rate.
    #[must_use]
    pub const fn requested_fee_rate(&self) -> LabFeeRate {
        self.requested_fee_rate
    }

    /// Returns the planned transactions in execution order.
    #[must_use]
    pub fn transactions(&self) -> &[PlannedTransaction] {
        &self.transactions
    }

    /// Returns mutable planned transactions for signing and durable recovery updates.
    #[must_use]
    pub fn transactions_mut(&mut self) -> &mut [PlannedTransaction] {
        &mut self.transactions
    }

    /// Returns every input outpoint reserved by this plan.
    #[must_use]
    pub fn reserved_outpoints(&self) -> Vec<OutPoint> {
        self.input_outpoints().collect()
    }

    /// Locks every planned input and persists the BDK wallet change.
    ///
    /// Calling this more than once is safe and restores locks after an interrupted prepare step.
    ///
    /// # Errors
    ///
    /// Returns an error when BDK cannot persist the reservations.
    pub fn reserve_inputs(&self, wallet: &mut WalletAccount) -> Result<(), BulkPlanError> {
        for outpoint in self.input_outpoints() {
            wallet.bdk_mut().lock_outpoint(outpoint);
        }
        wallet.persist().map_err(BulkPlanError::Wallet)?;

        Ok(())
    }

    /// Unlocks every input reserved for this plan and persists the change.
    ///
    /// # Errors
    ///
    /// Returns an error when the BDK wallet cannot persist the unlocks.
    pub fn release_reservations(&self, wallet: &mut WalletAccount) -> Result<(), BulkPlanError> {
        for outpoint in self.input_outpoints() {
            wallet.bdk_mut().unlock_outpoint(outpoint);
        }
        wallet.persist().map_err(BulkPlanError::Wallet)?;

        Ok(())
    }

    fn input_outpoints(&self) -> impl Iterator<Item = OutPoint> + '_ {
        self.transactions
            .iter()
            .flat_map(PlannedTransaction::input_outpoints)
    }
}

/// Builds BDK PSBTs for one wallet account.
pub struct BulkPlanner<'a> {
    wallet: &'a mut WalletAccount,
}

impl<'a> BulkPlanner<'a> {
    /// Creates a planner for one address-type account.
    pub const fn new(wallet: &'a mut WalletAccount) -> Self {
        Self { wallet }
    }

    /// Builds all PSBTs without reserving their inputs.
    ///
    /// Independent batch transactions never spend each other's change.
    ///
    /// # Errors
    ///
    /// Returns an error for insufficient funds or invalid selected inputs.
    pub fn plan(
        &mut self,
        request: &BulkRequest,
        fee_rate: LabFeeRate,
    ) -> Result<BulkPlan, BulkPlanError> {
        let transactions = match &request.0 {
            BulkRequestKind::FanOut {
                amount_per_output,
                destinations,
            } => {
                vec![self.plan_fan_out(*amount_per_output, destinations, fee_rate)?]
            }
            BulkRequestKind::Batch {
                amount_per_transaction,
                destinations,
            } => self.plan_batch(*amount_per_transaction, destinations, fee_rate)?,
            BulkRequestKind::Consolidate {
                destination,
                selected_inputs,
            } => {
                vec![self.plan_consolidation(destination, selected_inputs.as_deref(), fee_rate)?]
            }
        };
        let plan = BulkPlan {
            kind: request.kind(),
            requested_fee_rate: fee_rate,
            transactions,
        };

        ensure_unique_inputs(&plan)?;
        Ok(plan)
    }

    fn plan_fan_out(
        &mut self,
        amount: Amount,
        destinations: &DestinationScripts,
        fee_rate: LabFeeRate,
    ) -> Result<PlannedTransaction, BulkPlanError> {
        let recipients = destinations
            .0
            .iter()
            .cloned()
            .map(|script| (script, amount))
            .collect::<Vec<_>>();

        self.build_standard(recipients, Vec::new(), fee_rate, 0)
    }

    fn plan_batch(
        &mut self,
        amount: Amount,
        destinations: &DestinationScripts,
        fee_rate: LabFeeRate,
    ) -> Result<Vec<PlannedTransaction>, BulkPlanError> {
        let mut reserved = Vec::new();
        let mut transactions = Vec::with_capacity(destinations.len());

        for (index, destination) in destinations.0.iter().cloned().enumerate() {
            let transaction = self.build_standard(
                vec![(destination, amount)],
                reserved.clone(),
                fee_rate,
                index as u64,
            )?;
            reserved.extend(transaction.input_outpoints());
            transactions.push(transaction);
        }

        Ok(transactions)
    }

    fn build_standard(
        &mut self,
        recipients: Vec<(ScriptBuf, Amount)>,
        unspendable: Vec<OutPoint>,
        fee_rate: LabFeeRate,
        seed: u64,
    ) -> Result<PlannedTransaction, BulkPlanError> {
        let sent_amount = recipients
            .iter()
            .try_fold(Amount::ZERO, |total, (_, amount)| {
                total
                    .checked_add(*amount)
                    .ok_or(BulkPlanError::AmountOverflow)
            })?;
        let mut builder = self
            .wallet
            .bdk_mut()
            .build_tx()
            .coin_selection(CoveDefaultCoinSelection::new(seed));
        builder
            .set_recipients(recipients)
            .unspendable(unspendable)
            .fee_rate(fee_rate.as_bitcoin())
            .add_global_xpubs();
        let psbt = builder
            .finish()
            .map_err_str(BulkPlanError::CreateTransaction)?;

        self.preview(psbt, sent_amount)
    }

    fn plan_consolidation(
        &mut self,
        destination: &ScriptBuf,
        selected_inputs: Option<&[OutPoint]>,
        fee_rate: LabFeeRate,
    ) -> Result<PlannedTransaction, BulkPlanError> {
        let mut builder = self
            .wallet
            .bdk_mut()
            .build_tx()
            .coin_selection(CoveDefaultCoinSelection::new(0));
        builder
            .drain_to(destination.clone())
            .fee_rate(fee_rate.as_bitcoin())
            .add_global_xpubs();

        match selected_inputs {
            Some(inputs) => {
                builder
                    .add_utxos(inputs)
                    .map_err_str(BulkPlanError::SelectedInput)?;
                builder.manually_selected_only();
            }
            None => {
                builder.drain_wallet();
            }
        }

        let psbt = builder
            .finish()
            .map_err_str(BulkPlanError::CreateTransaction)?;
        let sent_amount = psbt
            .unsigned_tx
            .output
            .iter()
            .find(|output| output.script_pubkey == *destination)
            .map_or(Amount::ZERO, |output| output.value);

        self.preview(psbt, sent_amount)
    }

    fn preview(
        &self,
        psbt: Psbt,
        sent_amount: Amount,
    ) -> Result<PlannedTransaction, BulkPlanError> {
        let fee = self
            .wallet
            .bdk()
            .calculate_fee(&psbt.unsigned_tx)
            .map_err_str(BulkPlanError::CalculateFee)?;
        let actual_fee_rate = fee / psbt.unsigned_tx.weight();

        Ok(PlannedTransaction {
            psbt,
            fee,
            actual_fee_rate,
            sent_amount,
        })
    }
}

fn ensure_unique_inputs(plan: &BulkPlan) -> Result<(), BulkPlanError> {
    let mut inputs = HashSet::new();
    if plan
        .input_outpoints()
        .all(|outpoint| inputs.insert(outpoint))
    {
        return Ok(());
    }

    Err(BulkPlanError::InputReusedAcrossBatch)
}

fn require_positive_amount(amount: Amount) -> Result<(), BulkPlanError> {
    if amount == Amount::ZERO {
        return Err(BulkPlanError::ZeroAmount);
    }

    Ok(())
}

mod psbt_serde {
    use std::str::FromStr as _;

    use bitcoin::Psbt;
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer>(psbt: &Psbt, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&psbt.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Psbt, D::Error> {
        let value = String::deserialize(deserializer)?;
        Psbt::from_str(&value).map_err(D::Error::custom)
    }
}

/// Bulk planning and reservation errors.
#[derive(Debug, thiserror::Error)]
pub enum BulkPlanError {
    /// No destination was provided.
    #[error("at least one destination is required")]
    NoDestinations,
    /// The fan-out limit was exceeded.
    #[error("{0} destinations exceed the limit of 10000")]
    TooManyDestinations(usize),
    /// The independent transaction limit was exceeded.
    #[error("{0} batch transactions exceed the limit of 1000")]
    TooManyBatchTransactions(usize),
    /// A recipient amount was zero.
    #[error("recipient amount must be greater than zero")]
    ZeroAmount,
    /// Recipient amounts overflowed Bitcoin's amount range.
    #[error("recipient amount total is too large")]
    AmountOverflow,
    /// Selected consolidation had no inputs.
    #[error("select at least one UTXO to consolidate")]
    NoSelectedInputs,
    /// Selected consolidation repeated an outpoint.
    #[error("selected consolidation inputs contain a duplicate")]
    DuplicateSelectedInput,
    /// A batch transaction reused an earlier input.
    #[error("independent batch transactions cannot reuse inputs")]
    InputReusedAcrossBatch,
    /// BDK rejected a selected UTXO.
    #[error("selected UTXO is invalid: {0}")]
    SelectedInput(String),
    /// BDK could not create a transaction.
    #[error("transaction creation failed: {0}")]
    CreateTransaction(String),
    /// BDK could not calculate the fee.
    #[error("fee calculation failed: {0}")]
    CalculateFee(String),
    /// BDK wallet persistence failed.
    #[error("wallet persistence failed: {0}")]
    Wallet(#[source] crate::WalletError),
}

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::{
        Amount, Transaction, TxOut, absolute::LockTime, transaction::Version,
    };

    use super::{BulkPlanner, BulkRequest, DestinationScripts};
    use crate::{
        AccountIndex, AddressType, LabFeeRate, LabNetwork, MnemonicSecret, NetworkWallet, SignerId,
    };

    const TEST_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn fan_out_uses_fractional_fee_and_reserves_inputs() {
        let temp = std::env::temp_dir().join(format!("signer-lab-bulk-{}", uuid::Uuid::new_v4()));
        let secret = MnemonicSecret::parse(TEST_MNEMONIC).expect("test mnemonic is valid");
        let mut wallet = NetworkWallet::open(
            &temp,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("test wallet opens");
        let account = wallet.account_mut(AddressType::NativeSegwit);
        let receive = account
            .bdk()
            .peek_address(bdk_wallet::KeychainKind::External, 0)
            .address;
        let funding = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: receive.script_pubkey(),
            }],
        };
        account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
        account.persist().expect("funded test wallet persists");

        let scripts = DestinationScripts::new(vec![receive.script_pubkey(); 3])
            .expect("test destinations are valid");
        let request = BulkRequest::fan_out(Amount::from_sat(5_000), scripts)
            .expect("test fan-out request is valid");
        let plan = BulkPlanner::new(account)
            .plan(&request, LabFeeRate::DEFAULT)
            .expect("funded test wallet can plan fan-out");

        assert_eq!(plan.transactions().len(), 1);
        assert_eq!(
            plan.transactions()[0].sent_amount(),
            Amount::from_sat(15_000)
        );
        assert_eq!(account.snapshot().locked_utxo_count, 0);
        plan.reserve_inputs(account)
            .expect("planned inputs can be reserved");
        assert_eq!(account.snapshot().locked_utxo_count, 1);
        plan.release_reservations(account)
            .expect("planned inputs can be released");
        assert_eq!(account.snapshot().locked_utxo_count, 0);

        let _ = std::fs::remove_dir_all(temp);
    }
}
