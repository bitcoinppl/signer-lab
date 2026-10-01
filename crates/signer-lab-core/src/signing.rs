use bdk_wallet::bitcoin::{Address, Amount, Psbt};
use serde::{Deserialize, Serialize};

use crate::{
    NetworkWallet, QrDensity, QrFrames, QrTransport, QrTransportError, SignReport, WalletError,
};

/// A decoded PSBT that has not yet passed through transaction review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPsbt(Psbt);

impl PendingPsbt {
    /// Wraps a PSBT decoded from BBQR, UR, a file, or pasted text.
    #[must_use]
    pub const fn new(psbt: Psbt) -> Self {
        Self(psbt)
    }

    /// Computes review facts and advances the PSBT to the reviewed state.
    ///
    /// # Errors
    ///
    /// Returns an error when transaction amounts overflow.
    pub fn review(self, wallet: &NetworkWallet) -> Result<ReviewedPsbt, PsbtReviewError> {
        let review = PsbtReview::from_psbt(&self.0, wallet)?;

        Ok(ReviewedPsbt {
            psbt: self.0,
            review,
        })
    }
}

/// User-visible output facts for PSBT review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PsbtOutputReview {
    /// Output index in the unsigned transaction.
    pub index: usize,
    /// Output amount.
    pub amount: Amount,
    /// Address text when the script is standard for the active network.
    pub address: Option<String>,
    /// True when one of this signer's BDK accounts owns the script.
    pub owned_by_signer: bool,
}

/// Review facts shown before a signing action is enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PsbtReview {
    /// Number of transaction inputs.
    pub input_count: usize,
    /// Number of inputs that include this signer's fingerprint.
    pub matching_input_count: usize,
    /// Total known input amount, or `None` when any previous output is absent.
    pub input_amount: Option<Amount>,
    /// Total output amount.
    pub output_amount: Amount,
    /// Exact fee when all input amounts are known.
    pub fee: Option<Amount>,
    /// Output details in transaction order.
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
                let owned_by_signer = [
                    crate::AddressType::Legacy,
                    crate::AddressType::WrappedSegwit,
                    crate::AddressType::NativeSegwit,
                ]
                .into_iter()
                .any(|address_type| {
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

/// A PSBT that has review facts attached and can now be signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewedPsbt {
    psbt: Psbt,
    review: PsbtReview,
}

impl ReviewedPsbt {
    /// Returns the immutable review facts.
    #[must_use]
    pub const fn review(&self) -> &PsbtReview {
        &self.review
    }

    /// Signs every matching input with the selected virtual signer.
    ///
    /// # Errors
    ///
    /// Returns an error when no input matches or BDK rejects signing.
    pub fn sign(
        mut self,
        wallet: &NetworkWallet,
        trust_witness_utxo: bool,
    ) -> Result<SignedPsbt, WalletError> {
        let report = wallet.sign(&mut self.psbt, trust_witness_utxo)?;

        Ok(SignedPsbt {
            psbt: self.psbt,
            review: self.review,
            report,
        })
    }
}

/// A signed PSBT ready for BBQR or `crypto-psbt` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPsbt {
    psbt: Psbt,
    review: PsbtReview,
    report: SignReport,
}

impl SignedPsbt {
    /// Returns the signed PSBT.
    #[must_use]
    pub const fn psbt(&self) -> &Psbt {
        &self.psbt
    }

    /// Returns the pre-sign review facts.
    #[must_use]
    pub const fn review(&self) -> &PsbtReview {
        &self.review
    }

    /// Returns BDK's signing result details.
    #[must_use]
    pub const fn report(&self) -> SignReport {
        self.report
    }

    /// Encodes the signed PSBT for animated QR display.
    ///
    /// # Errors
    ///
    /// Returns an error when QR framing fails.
    pub fn qr_frames(
        &self,
        transport: QrTransport,
        density: QrDensity,
    ) -> Result<QrFrames, QrTransportError> {
        QrFrames::from_psbt(&self.psbt, transport, density)
    }
}

fn input_amount(psbt: &Psbt) -> Result<Option<Amount>, PsbtReviewError> {
    let mut total = Amount::ZERO;

    for (unsigned_input, input) in psbt.unsigned_tx.input.iter().zip(&psbt.inputs) {
        let amount = if let Some(previous) = &input.witness_utxo {
            Some(previous.value)
        } else if let Some(previous_transaction) = &input.non_witness_utxo {
            previous_transaction
                .output
                .get(unsigned_input.previous_output.vout as usize)
                .map(|output| output.value)
        } else {
            None
        };
        let Some(amount) = amount else {
            return Ok(None);
        };
        total = total
            .checked_add(amount)
            .ok_or(PsbtReviewError::AmountOverflow)?;
    }

    Ok(Some(total))
}

/// PSBT review errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PsbtReviewError {
    /// Input or output amounts exceeded Bitcoin's amount range.
    #[error("PSBT amount total is too large")]
    AmountOverflow,
    /// Known outputs are larger than known inputs.
    #[error("PSBT outputs exceed its inputs")]
    OutputsExceedInputs,
}

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::{
        Amount, OutPoint, Psbt, ScriptBuf, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
        transaction::Version,
    };

    use super::PendingPsbt;
    use crate::{AccountIndex, LabNetwork, MnemonicSecret, NetworkWallet, SignerId};

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
}
