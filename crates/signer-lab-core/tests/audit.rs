use std::path::PathBuf;

use bitcoin::{
    Amount, OutPoint, Psbt, ScriptBuf, Transaction, TxIn, TxOut, absolute::LockTime,
    secp256k1::Secp256k1, transaction::Version,
};
use cove_ur::CryptoPsbt;
use signer_lab_core::{
    AccountIndex, AddressType, BulkPlanError, BulkPlanner, BulkRequest, DestinationScripts,
    LabFeeRate, LabNetwork, MnemonicSecret, NetworkWallet, PendingPsbt, PsbtReviewError, QrDensity,
    QrFrames, QrScanProgress, QrScanner, QrTransport, QrTransportError, SignerId,
};

struct TestWallet {
    wallet: NetworkWallet,
    root: PathBuf,
}

impl TestWallet {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("signer-lab-audit-{}", uuid::Uuid::new_v4()));
        let secret = MnemonicSecret::parse("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").expect("test mnemonic");
        let wallet = NetworkWallet::open(
            &root,
            SignerId::new(),
            LabNetwork::MutinyNet,
            AccountIndex::default(),
            &secret,
        )
        .expect("wallet opens");

        Self { wallet, root }
    }
}

impl Drop for TestWallet {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn transaction(inputs: usize, value: u64) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn::default(); inputs],
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

#[test]
fn review_rejects_inconsistent_maps_and_previous_outputs() {
    let fixture = TestWallet::new();
    let previous = transaction(0, 10_000);
    let mut unsigned = transaction(1, 9_000);
    unsigned.input[0].previous_output = OutPoint::new(previous.compute_txid(), 0);
    let mut psbt = Psbt::from_unsigned_tx(unsigned).expect("unsigned transaction");
    psbt.inputs[0].non_witness_utxo = Some(previous.clone());
    assert_eq!(
        PendingPsbt::new(psbt.clone())
            .review(&fixture.wallet)
            .expect("valid review")
            .review()
            .fee,
        Some(Amount::from_sat(1_000))
    );

    let mut invalid = psbt.clone();
    invalid.inputs.clear();
    assert!(matches!(
        PendingPsbt::new(invalid).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidStructure)
    ));

    let mut invalid = psbt.clone();
    invalid.outputs.clear();
    assert!(matches!(
        PendingPsbt::new(invalid).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidStructure)
    ));

    let mut invalid = psbt.clone();
    invalid.inputs[0].non_witness_utxo = Some(transaction(0, 20_000));
    assert!(matches!(
        PendingPsbt::new(invalid).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidPreviousOutput { index: 0 })
    ));

    let mut invalid = psbt.clone();
    invalid.unsigned_tx.input[0].previous_output.vout = 1;
    assert!(matches!(
        PendingPsbt::new(invalid).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidPreviousOutput { index: 0 })
    ));
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(20_000),
        script_pubkey: ScriptBuf::new(),
    });

    assert!(matches!(
        PendingPsbt::new(psbt).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidPreviousOutput { index: 0 })
    ));

    let mut psbt = Psbt::from_unsigned_tx(transaction(2, 0)).expect("unsigned transaction");
    psbt.inputs[1].non_witness_utxo = Some(previous);
    assert!(matches!(
        PendingPsbt::new(psbt).review(&fixture.wallet),
        Err(PsbtReviewError::InvalidPreviousOutput { index: 1 })
    ));
}

#[test]
fn qr_accepts_uppercase_ur_but_rejects_mixed_transports() {
    let psbt = Psbt::from_unsigned_tx(transaction(0, 0)).expect("unsigned transaction");
    let ur = CryptoPsbt::new(psbt.clone())
        .to_ur_string()
        .expect("single-part UR");

    let mut scanner = QrScanner::new();
    assert!(matches!(
        scanner.receive(&ur.to_ascii_uppercase()),
        Ok(QrScanProgress::Complete { .. })
    ));

    let frames =
        QrFrames::from_psbt(&psbt, QrTransport::Bbqr, QrDensity::default()).expect("BBQR frames");

    scanner.reset();
    scanner.receive(&frames.frames[0]).expect("BBQR frame");
    assert!(matches!(
        scanner.receive(&ur),
        Err(QrTransportError::MixedTransports)
    ));

    scanner.reset();
    scanner.receive(&ur).expect("UR frame");
    assert!(matches!(
        scanner.receive(&frames.frames[0]),
        Err(QrTransportError::MixedTransports)
    ));

    assert!(
        serde_json::from_str::<QrDensity>(r#"{"ur_fragment_len":0,"bbqr_version":15}"#).is_err()
    );
}

#[test]
fn selected_consolidation_does_not_spend_reserved_inputs() {
    let mut fixture = TestWallet::new();
    let account = fixture.wallet.account_mut(AddressType::NativeSegwit);
    let destination = account
        .bdk()
        .peek_address(bdk_wallet::KeychainKind::External, 0)
        .address
        .script_pubkey();

    let mut funding = transaction(0, 50_000);
    funding.output[0].script_pubkey = destination.clone();
    let outpoint = OutPoint::new(funding.compute_txid(), 0);
    account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
    account.bdk_mut().lock_outpoint(outpoint);
    account.persist().expect("funding and lock persist");
    let request =
        BulkRequest::consolidate_selected(destination, vec![outpoint]).expect("selected request");

    assert!(matches!(
        BulkPlanner::new(account).plan(&request, LabFeeRate::DEFAULT),
        Err(BulkPlanError::SelectedInput(_))
    ));
}

#[test]
fn fee_rate_rejects_arithmetic_overflow_at_construction() {
    assert!(LabFeeRate::from_sat_per_kwu(u64::MAX).is_err());
}

#[test]
fn serialized_bulk_requests_cannot_bypass_constructor_checks() {
    assert!(serde_json::from_str::<DestinationScripts>("[]").is_err());
    let request = BulkRequest::consolidate_selected(ScriptBuf::new(), vec![OutPoint::default()])
        .expect("request");

    let mut json = serde_json::to_value(request).expect("request serializes");
    json["selected_inputs"] = serde_json::json!([]);
    assert!(serde_json::from_value::<BulkRequest>(json).is_err());
}

#[test]
fn signing_counts_new_finalization_and_rejects_other_accounts() {
    let mut fixture = TestWallet::new();
    let account = fixture.wallet.account_mut(AddressType::NativeSegwit);
    let script = account
        .bdk()
        .peek_address(bdk_wallet::KeychainKind::External, 0)
        .address
        .script_pubkey();

    let mut funding = transaction(0, 50_000);
    funding.output[0].script_pubkey = script.clone();
    account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
    let request = BulkRequest::fan_out(
        Amount::from_sat(1_000),
        DestinationScripts::new(vec![script]).expect("destination"),
    )
    .expect("request");

    let plan = BulkPlanner::new(account)
        .plan(&request, LabFeeRate::DEFAULT)
        .expect("plan");

    let mut psbt = plan.transactions()[0].psbt().clone();
    let mut wrong_account = psbt.clone();
    for (_, path) in wrong_account.inputs[0].bip32_derivation.values_mut() {
        let mut components = path.as_ref().to_vec();
        components[2] = bitcoin::bip32::ChildNumber::from_hardened_idx(1).expect("account index");
        *path = components.into();
    }

    assert!(matches!(
        fixture.wallet.sign(&mut wrong_account, true),
        Err(signer_lab_core::WalletError::NoMatchingInputs)
    ));

    let secp = Secp256k1::new();
    let key = bitcoin::secp256k1::SecretKey::from_slice(&[1; 32]).expect("test key");
    let public =
        bitcoin::PublicKey::new(bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &key));

    let signature = secp.sign_ecdsa(&bitcoin::secp256k1::Message::from_digest([2; 32]), &key);
    psbt.inputs[0]
        .partial_sigs
        .insert(public, bitcoin::ecdsa::Signature::sighash_all(signature));

    let derivation = psbt.inputs[0].bip32_derivation.clone();
    let report = fixture.wallet.sign(&mut psbt, true).expect("signing");
    assert!(report.finalized);
    assert_eq!(report.signed_inputs, 1);
    psbt.inputs[0].bip32_derivation = derivation;
    assert_eq!(
        fixture
            .wallet
            .sign(&mut psbt, true)
            .expect("repeat signing")
            .signed_inputs,
        0
    );
}

#[test]
fn native_segwit_fee_estimate_matches_signed_weight() {
    let mut fixture = TestWallet::new();
    let account = fixture.wallet.account_mut(AddressType::NativeSegwit);
    let script = account
        .bdk()
        .peek_address(bdk_wallet::KeychainKind::External, 0)
        .address
        .script_pubkey();

    for amount in [60_000, 60_001] {
        let mut funding = transaction(0, amount);
        funding.output[0].script_pubkey = script.clone();
        account.bdk_mut().apply_unconfirmed_txs([(funding, 1)]);
    }

    account.persist().expect("funding persists");
    let request = BulkRequest::fan_out(
        Amount::from_sat(100_000),
        DestinationScripts::new(vec![script]).expect("scripts"),
    )
    .expect("request");

    let plan = BulkPlanner::new(account)
        .plan(&request, LabFeeRate::DEFAULT)
        .expect("plan");

    let planned = &plan.transactions()[0];
    let input_count = planned.psbt().unsigned_tx.input.len() as u64;
    assert_eq!(input_count, 2);
    let satisfaction = account
        .bdk()
        .public_descriptor(bdk_wallet::KeychainKind::External)
        .max_weight_to_satisfy()
        .expect("satisfaction weight");
    let estimated_weight = planned.psbt().unsigned_tx.weight()
        + satisfaction * input_count
        + bitcoin::Weight::from_wu(2 + input_count);
    assert_eq!(
        planned.estimated_fee_rate(),
        planned.fee() / estimated_weight
    );
    let mut psbt = planned.psbt().clone();
    assert!(
        fixture
            .wallet
            .sign(&mut psbt, true)
            .expect("sign")
            .finalized
    );

    let signed = psbt.extract_tx().expect("signed transaction");
    // miniscript budgets 72 bytes per signature including the sighash suffix;
    // low-R signatures use at most 71, with this fixture at most 2 WU below the budget per input
    let signature_shortfall: u64 = signed
        .input
        .iter()
        .map(|input| {
            let signature = input.witness.iter().next().expect("signature");
            72 - signature.len() as u64
        })
        .sum();
    assert!(signature_shortfall <= 2 * input_count);
    assert_eq!(
        estimated_weight.to_wu(),
        signed.weight().to_wu() + signature_shortfall
    );
    let json = serde_json::to_value(planned).expect("plan JSON");
    assert!(json.get("estimated_fee_rate").is_some());
    assert!(json.get("actual_fee_rate").is_none());
}
