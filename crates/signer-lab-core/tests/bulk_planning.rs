use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use bdk_wallet::KeychainKind;
use bitcoin::{Amount, Transaction, TxOut, absolute::LockTime, transaction::Version};
use signer_lab_core::{
    AddressType, BulkDestinationSpec, BulkPlanError, BulkPlanOperationError, BulkShape,
    ExternalDescriptor, InputReservation, LabFeeRate, LabNetwork, LabStore, MnemonicSecret,
    OutputCount, PlanBulk, SecretStore, SecretStoreError, SignerId, SignerRepository,
    VirtualSigner,
};

#[derive(Default)]
struct Secrets(Mutex<HashMap<SignerId, String>>);

impl SecretStore for Secrets {
    fn save(&self, id: SignerId, secret: &MnemonicSecret) -> Result<(), SecretStoreError> {
        self.0
            .lock()
            .expect("lock")
            .insert(id, secret.expose_phrase().to_owned());
        Ok(())
    }

    fn load(&self, id: SignerId) -> Result<MnemonicSecret, SecretStoreError> {
        MnemonicSecret::parse(self.0.lock().expect("lock").get(&id).expect("secret"))
            .map_err(SecretStoreError::InvalidSecret)
    }

    fn delete(&self, id: SignerId) -> Result<(), SecretStoreError> {
        self.0.lock().expect("lock").remove(&id);
        Ok(())
    }
}

struct Fixture {
    repository: SignerRepository<Secrets>,
    source: VirtualSigner,
    destination: VirtualSigner,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("signer-lab-plan-{}", uuid::Uuid::new_v4()));
        let store = LabStore::create(root.join("metadata.db"))
            .await
            .expect("store");

        let mut repository = SignerRepository::new(store, Secrets::default(), root.join("wallets"));
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let source = repository
            .import("source", 0, phrase)
            .await
            .expect("source");

        let destination = repository
            .import("destination", 1, phrase)
            .await
            .expect("destination");

        let mut wallet = repository
            .open_wallet(&source, LabNetwork::MutinyNet)
            .expect("wallet");

        let account = wallet.account_mut(AddressType::NativeSegwit);
        let script = account
            .bdk()
            .peek_address(KeychainKind::External, 0)
            .address
            .script_pubkey();
        account.bdk_mut().apply_unconfirmed_txs([(
            Transaction {
                version: Version::TWO,
                lock_time: LockTime::ZERO,
                input: Vec::new(),
                output: vec![TxOut {
                    value: Amount::from_sat(100_000),
                    script_pubkey: script,
                }],
            },
            1,
        )]);

        account.persist().expect("funding persists");
        Self {
            repository,
            source,
            destination,
            root,
        }
    }

    fn request(&self, shape: BulkShape) -> PlanBulk {
        PlanBulk {
            signer: self.source.clone(),
            network: LabNetwork::MutinyNet,
            shape,
            destination: BulkDestinationSpec::SavedSigner {
                signer_id: self.destination.id,
                address_type: AddressType::WrappedSegwit,
            },
            fee_rate: LabFeeRate::DEFAULT,
        }
    }

    fn next_index(&self, signer: &VirtualSigner) -> u32 {
        self.repository
            .open_wallet(signer, LabNetwork::MutinyNet)
            .expect("wallet")
            .account(AddressType::WrappedSegwit)
            .bdk()
            .next_derivation_index(KeychainKind::External)
    }

    fn locks(&self) -> usize {
        self.repository
            .open_wallet(&self.source, LabNetwork::MutinyNet)
            .expect("wallet")
            .account(AddressType::NativeSegwit)
            .snapshot()
            .locked_utxo_count
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn invalid_counts_and_amounts_do_not_reveal_saved_addresses() {
    let fixture = Fixture::new().await;
    let before = fixture.next_index(&fixture.destination);
    assert!(matches!(
        OutputCount::try_from(10_001),
        Err(BulkPlanError::TooManyDestinations(10_001))
    ));

    assert!(OutputCount::try_from(0).is_err());
    assert!(matches!(
        BulkShape::batch(
            OutputCount::try_from(1_001).expect("bounded count"),
            Amount::ONE_SAT
        ),
        Err(BulkPlanError::TooManyBatchTransactions(1_001))
    ));

    assert!(matches!(
        BulkShape::fan_out(OutputCount::try_from(1).expect("count"), Amount::ZERO),
        Err(BulkPlanError::ZeroAmount)
    ));

    for construct in [BulkShape::fan_out, BulkShape::batch] {
        assert!(matches!(
            construct(
                OutputCount::try_from(2).expect("count"),
                Amount::from_sat(u64::MAX)
            ),
            Err(BulkPlanError::AmountOverflow)
        ));
    }

    assert_eq!(fixture.next_index(&fixture.destination), before);
}

#[tokio::test]
async fn source_destination_is_rejected_without_reveal() {
    let mut fixture = Fixture::new().await;
    let before = fixture.next_index(&fixture.source);
    let mut request = fixture.request(BulkShape::consolidate());
    request.destination = BulkDestinationSpec::SavedSigner {
        signer_id: fixture.source.id,
        address_type: AddressType::WrappedSegwit,
    };

    assert!(matches!(
        fixture.repository.plan_bulk_operation(request).await,
        Err(BulkPlanOperationError::SourceIsDestination)
    ));

    assert_eq!(fixture.next_index(&fixture.source), before);
}

#[tokio::test]
async fn saved_operation_owns_reported_reservations() {
    let mut fixture = Fixture::new().await;
    fixture
        .repository
        .open_wallet(&fixture.destination, LabNetwork::MutinyNet)
        .expect("destination wallet")
        .account_mut(AddressType::WrappedSegwit)
        .reveal_receive_address()
        .expect("initial allocation");
    let before = fixture.next_index(&fixture.destination);
    let request = fixture.request(
        BulkShape::fan_out(
            OutputCount::try_from(2).expect("count"),
            Amount::from_sat(5_000),
        )
        .expect("valid shape"),
    );

    let result = fixture
        .repository
        .plan_bulk_operation(request)
        .await
        .expect("plan");

    assert_eq!(result.reservation, InputReservation::Held);
    assert_eq!(result.destination_range, None);
    let saved = fixture
        .repository
        .current_bulk_operation(fixture.source.id, LabNetwork::MutinyNet)
        .await
        .expect("load")
        .expect("saved");

    let signer_lab_core::CurrentBulkOperation::Active { operation_id, plan } = saved else {
        panic!("active operation");
    };
    assert_eq!(operation_id, result.operation_id);
    assert_eq!(plan, result.plan);
    assert_eq!(fixture.locks(), result.plan.reserved_outpoints().len());
    assert_eq!(fixture.next_index(&fixture.destination), before + 2);
    let next = fixture.request(BulkShape::consolidate());
    assert!(matches!(
        fixture.repository.plan_bulk_operation(next).await,
        Err(BulkPlanOperationError::Repository(
            signer_lab_core::RepositoryError::ActiveBulkOperation(_)
        ))
    ));

    assert_eq!(fixture.next_index(&fixture.destination), before + 2);
}

#[tokio::test]
async fn reservation_failure_returns_the_saved_operation_for_recovery() {
    let mut fixture = Fixture::new().await;
    let path = fixture
        .root
        .join("wallets")
        .join(fixture.source.id.to_string())
        .join("mutinynet/bip84.sqlite");

    let connection = bdk_wallet::rusqlite::Connection::open(path).expect("wallet database");
    connection.execute_batch("CREATE TRIGGER reject_lock BEFORE INSERT ON bdk_wallet_locked_outpoints BEGIN SELECT RAISE(FAIL, 'test reservation failure'); END;").expect("trigger");
    let request = fixture.request(BulkShape::consolidate());
    let result = fixture
        .repository
        .plan_bulk_operation(request)
        .await
        .expect("operation is saved despite lock failure");

    assert!(matches!(result.reservation, InputReservation::Failed(_)));
    assert_eq!(fixture.locks(), 0);
    let saved = fixture
        .repository
        .current_bulk_operation(fixture.source.id, LabNetwork::MutinyNet)
        .await
        .expect("saved plan")
        .expect("operation exists");

    let signer_lab_core::CurrentBulkOperation::Active { operation_id, plan } = saved else {
        panic!("active operation");
    };
    assert_eq!(operation_id, result.operation_id);
    assert_eq!(plan, result.plan);
    connection
        .execute_batch("DROP TRIGGER reject_lock")
        .expect("restore persistence");

    fixture
        .repository
        .reconcile_bulk_reservations(&fixture.source, LabNetwork::MutinyNet)
        .await
        .expect("recover locks");

    assert_eq!(fixture.locks(), result.plan.reserved_outpoints().len());
}

#[tokio::test]
async fn external_derivation_ranges_are_checked_before_wallet_changes() {
    let mut fixture = Fixture::new().await;
    let wallet = fixture
        .repository
        .open_wallet(&fixture.destination, LabNetwork::MutinyNet)
        .expect("destination wallet");

    let public = wallet.public_accounts().expect("accounts").remove(2);
    let descriptor = ExternalDescriptor::parse(&public.external_descriptor, LabNetwork::MutinyNet)
        .expect("descriptor");

    drop(wallet);
    let orphan = bitcoin::OutPoint::default();
    let mut source_wallet = fixture
        .repository
        .open_wallet(&fixture.source, LabNetwork::MutinyNet)
        .expect("source wallet");

    let account = source_wallet.account_mut(AddressType::NativeSegwit);
    account.bdk_mut().lock_outpoint(orphan);
    account.persist().expect("orphan persists");
    drop(source_wallet);
    for (start_index, count) in [((1 << 31) - 1, 2), (1 << 31, 1), (u32::MAX, 2)] {
        for destination in [
            BulkDestinationSpec::AccountXpub {
                xpub: public.account_xpub,
                address_type: AddressType::NativeSegwit,
                start_index,
            },
            BulkDestinationSpec::Descriptor {
                descriptor: descriptor.clone(),
                start_index,
            },
        ] {
            let mut request = fixture.request(
                BulkShape::fan_out(
                    OutputCount::try_from(count).expect("count"),
                    Amount::from_sat(5_000),
                )
                .expect("valid shape"),
            );

            request.destination = destination;
            assert!(matches!(
                fixture.repository.plan_bulk_operation(request).await,
                Err(BulkPlanOperationError::Destination(
                    signer_lab_core::DestinationError::IndexTooLarge
                ))
            ));

            let wallet = fixture
                .repository
                .open_wallet(&fixture.source, LabNetwork::MutinyNet)
                .expect("source");

            assert!(
                wallet
                    .account(AddressType::NativeSegwit)
                    .bdk()
                    .is_outpoint_locked(orphan)
            );
        }
    }

    assert!(
        fixture
            .repository
            .bulk_operations(fixture.source.id)
            .await
            .expect("operations")
            .is_empty()
    );
}

#[tokio::test]
async fn external_start_indexes_select_scripts_and_report_the_range() {
    let mut fixture = Fixture::new().await;
    let wallet = fixture
        .repository
        .open_wallet(&fixture.destination, LabNetwork::MutinyNet)
        .expect("destination wallet");

    let public = wallet.public_accounts().expect("accounts").remove(2);
    let scripts: Vec<_> = (7..=8)
        .map(|index| {
            wallet
                .account(AddressType::NativeSegwit)
                .bdk()
                .peek_address(KeychainKind::External, index)
                .address
                .script_pubkey()
        })
        .collect();
    drop(wallet);
    let descriptor = ExternalDescriptor::parse(&public.external_descriptor, LabNetwork::MutinyNet)
        .expect("descriptor");

    for destination in [
        BulkDestinationSpec::AccountXpub {
            xpub: public.account_xpub,
            address_type: AddressType::NativeSegwit,
            start_index: 7,
        },
        BulkDestinationSpec::Descriptor {
            descriptor,
            start_index: 7,
        },
    ] {
        let mut request = fixture.request(
            BulkShape::fan_out(
                OutputCount::try_from(2).expect("count"),
                Amount::from_sat(5_000),
            )
            .expect("valid shape"),
        );

        request.destination = destination;
        let result = fixture
            .repository
            .plan_bulk_operation(request)
            .await
            .expect("plan");

        assert_eq!(result.destination_range, Some(7..=8));
        let (_, saved) = fixture
            .repository
            .saved_bulk_plan(fixture.source.id, result.operation_id)
            .await
            .expect("saved plan");
        assert_eq!(saved.destination_range(), Some(&(7..=8)));
        for script in &scripts {
            assert!(
                result.plan.transactions()[0]
                    .psbt()
                    .unsigned_tx
                    .output
                    .iter()
                    .any(|output| output.script_pubkey == *script
                        && output.value == Amount::from_sat(5_000))
            );
        }

        fixture
            .repository
            .cancel_bulk_operation(&fixture.source, LabNetwork::MutinyNet, result.operation_id)
            .await
            .expect("cancel");
    }

    assert_eq!(fixture.next_index(&fixture.destination), 0);
}

#[tokio::test]
async fn baseline_saved_plans_load_and_reconcile() {
    let mut fixture = Fixture::new().await;
    let request = fixture.request(BulkShape::consolidate());
    let operation = fixture
        .repository
        .plan_bulk_operation(request)
        .await
        .expect("plan");
    let mut encoded = serde_json::to_value(&operation.plan).expect("saved plan");
    encoded
        .as_object_mut()
        .expect("plan object")
        .remove("destination_range");
    for transaction in encoded["transactions"]
        .as_array_mut()
        .expect("transactions")
    {
        let transaction = transaction.as_object_mut().expect("transaction");
        let rate = transaction.remove("estimated_fee_rate").expect("fee rate");
        transaction.insert("actual_fee_rate".to_owned(), rate);
    }

    let loaded: signer_lab_core::BulkPlan =
        serde_json::from_value(encoded.clone()).expect("baseline plan");
    assert_eq!(loaded.destination_range(), None);
    assert_eq!(loaded.transactions(), operation.plan.transactions());
    let mut stored = fixture
        .repository
        .bulk_operations(fixture.source.id)
        .await
        .expect("operations")
        .remove(0);
    stored.plan["plan"] = encoded;
    stored
        .plan
        .as_object_mut()
        .expect("stored plan")
        .remove("acknowledged_at");
    let mut store = LabStore::open(fixture.root.join("metadata.db"))
        .await
        .expect("store");
    store.save_operation(&stored).await.expect("save baseline");
    let current = fixture
        .repository
        .current_bulk_operation(fixture.source.id, LabNetwork::MutinyNet)
        .await
        .expect("load baseline")
        .expect("active plan");
    assert!(
        matches!(current, signer_lab_core::CurrentBulkOperation::Active { plan, .. } if plan == loaded)
    );
    let mut wallet = fixture
        .repository
        .open_wallet(&fixture.source, LabNetwork::MutinyNet)
        .expect("wallet");
    let account = wallet.account_mut(AddressType::NativeSegwit);
    for outpoint in loaded.reserved_outpoints() {
        account.bdk_mut().unlock_outpoint(outpoint);
    }
    account.persist().expect("clear locks");
    drop(wallet);
    assert_eq!(fixture.locks(), 0);
    fixture
        .repository
        .reconcile_bulk_reservations(&fixture.source, LabNetwork::MutinyNet)
        .await
        .expect("reconcile baseline");
    assert_eq!(fixture.locks(), loaded.reserved_outpoints().len());
}

#[tokio::test]
async fn rejected_plans_do_not_allocate_saved_signer_addresses() {
    let mut fixture = Fixture::new().await;
    for amount in [Amount::ONE_SAT, Amount::from_sat(100_000)] {
        let before = fixture.next_index(&fixture.destination);
        let request = fixture.request(
            BulkShape::fan_out(OutputCount::try_from(3).expect("count"), amount).expect("shape"),
        );
        assert!(matches!(
            fixture.repository.plan_bulk_operation(request).await,
            Err(BulkPlanOperationError::Plan(_))
        ));
        assert_eq!(fixture.next_index(&fixture.destination), before);
        assert!(
            fixture
                .repository
                .current_bulk_operation(fixture.source.id, LabNetwork::MutinyNet)
                .await
                .expect("load")
                .is_none()
        );
        assert_eq!(fixture.locks(), 0);
    }
}
