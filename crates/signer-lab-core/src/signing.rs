use bdk_wallet::bitcoin::{Address, Amount, Psbt, Transaction};
use serde::{Deserialize, Serialize};

use crate::{
    NetworkWallet, QrDensity, QrFrames, QrTransport, QrTransportError, SignReport, WalletError,
};

/// A decoded PSBT that has not yet passed through transaction review
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPsbt(Psbt);

impl PendingPsbt {
    /// Wraps a PSBT decoded from BBQR, UR, a file, or pasted text
    #[must_use]
    pub const fn new(psbt: Psbt) -> Self {
        Self(psbt)
    }

    /// Computes review facts and advances the PSBT to the reviewed state
    ///
    /// # Errors
    ///
    /// Returns an error for inconsistent PSBT maps, previous outputs, or invalid amount totals
    pub fn review(self, wallet: &NetworkWallet) -> Result<ReviewedPsbt, PsbtReviewError> {
        let review = PsbtReview::from_psbt(&self.0, wallet)?;

        Ok(ReviewedPsbt {
            psbt: self.0,
            review,
        })
    }
}

/// User-visible output facts for PSBT review
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PsbtOutputReview {
    /// Output index in the unsigned transaction
    pub index: usize,
    /// Output amount
    pub amount: Amount,
    /// Address text when the script is standard for the active network
    pub address: Option<String>,
    /// True when one of this signer's BDK accounts owns the script
    pub owned_by_signer: bool,
}

/// Review facts shown before a signing action is enabled
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PsbtReview {
    /// Number of transaction inputs
    pub input_count: usize,
    /// Number of inputs that include this signer's fingerprint
    pub matching_input_count: usize,
    /// Total known input amount, or `None` when any previous output is absent
    pub input_amount: Option<Amount>,
    /// Total output amount
    pub output_amount: Amount,
    /// Exact fee when all input amounts are known
    pub fee: Option<Amount>,
    /// Output details in transaction order
    pub outputs: Vec<PsbtOutputReview>,
}

impl PsbtReview {
    fn from_psbt(psbt: &Psbt, wallet: &NetworkWallet) -> Result<Self, PsbtReviewError> {
        let input_amount = input_amount(psbt)?;
        let output_amount =
            psbt.unsigned_tx
                .output
                .iter()
                .try_fold(Amount::ZERO, |total, output| {
                    total
                        .checked_add(output.value)
                        .ok_or(PsbtReviewError::AmountOverflow)
                })?;

        let fee = input_amount
            .map(|input| {
                input
                    .checked_sub(output_amount)
                    .ok_or(PsbtReviewError::OutputsExceedInputs)
            })
            .transpose()?;

        let outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .enumerate()
            .map(|(index, output)| {
                let address =
                    Address::from_script(&output.script_pubkey, wallet.network.bitcoin_network())
                        .ok()
                        .map(|address| address.to_string());

                let owned_by_signer = crate::AddressType::ALL.into_iter().any(|address_type| {
                    wallet
                        .account(address_type)
                        .bdk()
                        .is_mine(output.script_pubkey.clone())
                });

                PsbtOutputReview {
                    index,
                    amount: output.value,
                    address,
                    owned_by_signer,
                }
            })
            .collect();

        let fingerprint = wallet.fingerprint();
        let matching_input_count = psbt
            .inputs
            .iter()
            .filter(|input| {
                input
                    .bip32_derivation
                    .values()
                    .any(|(origin, _)| *origin == fingerprint)
                    || input
                        .tap_key_origins
                        .values()
                        .any(|(_, (origin, _))| *origin == fingerprint)
            })
            .count();

        Ok(Self {
            input_count: psbt.inputs.len(),
            matching_input_count,
            input_amount,
            output_amount,
            fee,
            outputs,
        })
    }
}

/// A PSBT that has review facts attached and can now be signed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewedPsbt {
    psbt: Psbt,
    review: PsbtReview,
}

impl ReviewedPsbt {
    /// Returns the immutable review facts
    #[must_use]
    pub const fn review(&self) -> &PsbtReview {
        &self.review
    }

    /// Signs every matching input with the selected virtual signer
    ///
    /// # Errors
    ///
    /// Returns an error when no input matches or BDK rejects signing
    pub fn sign(
        mut self,
        wallet: &NetworkWallet,
        trust_witness_utxo: bool,
    ) -> Result<SignedPsbt, WalletError> {
        let report = wallet.sign(&mut self.psbt, trust_witness_utxo)?;
        let transaction = finalized_transaction(&self.psbt);

        Ok(SignedPsbt {
            psbt: self.psbt,
            review: self.review,
            report,
            transaction,
        })
    }
}

/// A signed PSBT ready for BBQR or `crypto-psbt` output
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPsbt {
    psbt: Psbt,
    review: PsbtReview,
    report: SignReport,
    transaction: Option<Transaction>,
}

impl SignedPsbt {
    /// Returns the signed PSBT
    #[must_use]
    pub const fn psbt(&self) -> &Psbt {
        &self.psbt
    }

    /// Returns the pre-sign review facts
    #[must_use]
    pub const fn review(&self) -> &PsbtReview {
        &self.review
    }

    /// Returns BDK's signing result details
    #[must_use]
    pub const fn report(&self) -> SignReport {
        self.report
    }

    /// Returns the network-ready transaction, or `None` while any input still lacks a final script
    #[must_use]
    pub const fn transaction(&self) -> Option<&Transaction> {
        self.transaction.as_ref()
    }

    /// Encodes the signed PSBT for animated QR display
    ///
    /// # Errors
    ///
    /// Returns an error when QR framing fails
    pub fn qr_frames(
        &self,
        transport: QrTransport,
        density: QrDensity,
    ) -> Result<QrFrames, QrTransportError> {
        QrFrames::from_psbt(&self.psbt, transport, density)
    }
}

/// Extracts the transaction only when every input is final, since a partial one cannot be broadcast
fn finalized_transaction(psbt: &Psbt) -> Option<Transaction> {
    let finalized = psbt
        .inputs
        .iter()
        .all(|input| input.final_script_sig.is_some() || input.final_script_witness.is_some());

    // review already showed the fee, and test transactions may pay high fees on purpose
    finalized.then(|| psbt.clone().extract_tx_unchecked_fee_rate())
}

fn input_amount(psbt: &Psbt) -> Result<Option<Amount>, PsbtReviewError> {
    if psbt.inputs.len() != psbt.unsigned_tx.input.len()
        || psbt.outputs.len() != psbt.unsigned_tx.output.len()
    {
        return Err(PsbtReviewError::InvalidStructure);
    }

    let mut total = Amount::ZERO;
    let mut all_known = true;

    for (index, (unsigned_input, input)) in
        psbt.unsigned_tx.input.iter().zip(&psbt.inputs).enumerate()
    {
        let previous_output = if let Some(transaction) = &input.non_witness_utxo {
            if transaction.compute_txid() != unsigned_input.previous_output.txid {
                return Err(PsbtReviewError::InvalidPreviousOutput { index });
            }

            let output = transaction
                .output
                .get(unsigned_input.previous_output.vout as usize)
                .ok_or(PsbtReviewError::InvalidPreviousOutput { index })?;

            if input
                .witness_utxo
                .as_ref()
                .is_some_and(|witness| witness != output)
            {
                return Err(PsbtReviewError::InvalidPreviousOutput { index });
            }

            Some(output)
        } else {
            input.witness_utxo.as_ref()
        };

        let Some(output) = previous_output else {
            all_known = false;
            continue;
        };

        total = total
            .checked_add(output.value)
            .ok_or(PsbtReviewError::AmountOverflow)?;
    }

    Ok(all_known.then_some(total))
}

/// PSBT review errors
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PsbtReviewError {
    /// PSBT maps do not match the unsigned transaction
    #[error("PSBT input or output maps do not match its transaction")]
    InvalidStructure,
    /// Previous transaction data does not match the referenced output
    #[error("PSBT input {index} has inconsistent previous output data")]
    InvalidPreviousOutput {
        /// Input with inconsistent previous output data
        index: usize,
    },
    /// Input or output amounts exceeded Bitcoin's amount range
    #[error("PSBT amount total is too large")]
    AmountOverflow,
    /// Known outputs are larger than known inputs
    #[error("PSBT outputs exceed its inputs")]
    OutputsExceedInputs,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bdk_wallet::{
        KeychainKind,
        bitcoin::{
            Amount, OutPoint, Psbt, ScriptBuf, Transaction, TxIn, TxOut, Witness,
            absolute::LockTime, transaction::Version,
        },
    };

    use super::PendingPsbt;
    use crate::{AccountIndex, AddressType, LabNetwork, MnemonicSecret, NetworkWallet, SignerId};

    const ALICE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const BOB: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

    #[test]
    fn review_calculates_fee_from_witness_utxo() {
        let temp = std::env::temp_dir().join(format!("signer-lab-review-{}", uuid::Uuid::new_v4()));
        let secret = MnemonicSecret::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .expect("test mnemonic is valid");

        let wallet = NetworkWallet::open(
            &temp,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("test wallet opens");

        let transaction = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::default(),
                script_sig: ScriptBuf::new(),
                sequence: bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(9_000),
                script_pubkey: ScriptBuf::new(),
            }],
        };

        let mut psbt = Psbt::from_unsigned_tx(transaction).expect("test transaction is valid");
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: ScriptBuf::new(),
        });

        let reviewed = PendingPsbt::new(psbt)
            .review(&wallet)
            .expect("test PSBT review succeeds");

        assert_eq!(reviewed.review().fee, Some(Amount::from_sat(1_000)));
        assert_eq!(reviewed.review().matching_input_count, 0);

        let _ = std::fs::remove_dir_all(temp);
    }

    fn open_wallet(temp: &Path, words: &str) -> NetworkWallet {
        let secret = MnemonicSecret::parse(words).expect("test mnemonic is valid");

        NetworkWallet::open(
            temp,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("test wallet opens")
    }

    /// Funds the wallet's first receive address and spends it to its next one
    fn owned_psbt(wallet: &mut NetworkWallet) -> Psbt {
        let account = wallet.account_mut(AddressType::NativeSegwit);
        let receive = account
            .bdk()
            .peek_address(KeychainKind::External, 0)
            .address
            .script_pubkey();
        let destination = account
            .bdk()
            .peek_address(KeychainKind::External, 1)
            .address
            .script_pubkey();

        let funding = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(50_000),
                script_pubkey: receive,
            }],
        };

        account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
        let mut builder = account.bdk_mut().build_tx();
        builder.add_recipient(destination, Amount::from_sat(20_000));
        builder.fee_absolute(Amount::from_sat(500));
        builder.finish().expect("owned PSBT builds")
    }

    #[test]
    fn signing_every_input_yields_the_broadcast_transaction() {
        let temp = std::env::temp_dir().join(format!("signer-lab-final-{}", uuid::Uuid::new_v4()));
        let mut wallet = open_wallet(&temp, ALICE);
        let psbt = owned_psbt(&mut wallet);
        let unsigned_txid = psbt.unsigned_tx.compute_txid();

        let signed = PendingPsbt::new(psbt)
            .review(&wallet)
            .expect("review")
            .sign(&wallet, true)
            .expect("sign");

        let transaction = signed.transaction().expect("every input is final");
        assert_eq!(transaction.compute_txid(), unsigned_txid);
        assert!(
            transaction
                .input
                .iter()
                .all(|input| !input.witness.is_empty())
        );

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn signing_some_inputs_has_no_transaction() {
        let temp =
            std::env::temp_dir().join(format!("signer-lab-partial-{}", uuid::Uuid::new_v4()));
        let mut wallet = open_wallet(&temp, ALICE);
        let mut other = open_wallet(&temp.join("other"), BOB);
        let mut psbt = owned_psbt(&mut wallet);
        let foreign = owned_psbt(&mut other);
        psbt.unsigned_tx
            .input
            .push(foreign.unsigned_tx.input[0].clone());
        psbt.inputs.push(foreign.inputs[0].clone());

        let signed = PendingPsbt::new(psbt)
            .review(&wallet)
            .expect("review")
            .sign(&wallet, true)
            .expect("sign");

        assert_eq!(signed.report().signed_inputs, 1);
        assert_eq!(signed.transaction(), None);

        let _ = std::fs::remove_dir_all(temp);
    }
}
