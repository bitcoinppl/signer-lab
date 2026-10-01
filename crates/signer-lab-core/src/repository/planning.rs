use std::{num::NonZeroUsize, ops::RangeInclusive};

use bitcoin::{Amount, ScriptBuf, bip32::Xpub};

use super::{BulkExecutionError, CurrentBulkOperation, RepositoryError, SignerRepository};
use crate::{
    AddressType, BulkPlan, BulkPlanError, BulkPlanner, BulkRequest, Destination, DestinationError,
    DestinationScripts, ExternalDescriptor, FixedAddress, LabFeeRate, LabNetwork, NetworkWallet,
    OperationId, OperationKind, SecretStore, SignerId, VirtualSigner, WalletError,
};

/// A nonzero destination count bounded by the fan-out limit
///
/// Batch requests also check the lower batch limit before resolving destinations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCount(NonZeroUsize);

impl TryFrom<usize> for OutputCount {
    type Error = BulkPlanError;

    fn try_from(count: usize) -> Result<Self, Self::Error> {
        if count > DestinationScripts::MAX_COUNT {
            return Err(BulkPlanError::TooManyDestinations(count));
        }

        NonZeroUsize::new(count)
            .map(Self)
            .ok_or(BulkPlanError::NoDestinations)
    }
}

impl OutputCount {
    /// Returns the checked count
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// The transaction layout and recipient amounts
#[derive(Debug, Clone, Copy)]
pub struct BulkShape(BulkShapeKind);

#[derive(Debug, Clone, Copy)]
enum BulkShapeKind {
    FanOut { count: OutputCount, amount: Amount },
    Batch { count: OutputCount, amount: Amount },
    Consolidate,
}

impl BulkShape {
    /// Creates one transaction with a checked output count and amount
    ///
    /// # Errors
    /// Returns an error for zero amounts or overflowing recipient totals
    pub fn fan_out(count: OutputCount, amount: Amount) -> Result<Self, BulkPlanError> {
        Self::check_amount(count, amount)?;
        Ok(Self(BulkShapeKind::FanOut { count, amount }))
    }

    /// Creates independent transactions with checked recipient amounts
    ///
    /// # Errors
    /// Returns an error for excessive batch counts, zero amounts, or overflowing totals
    pub fn batch(count: OutputCount, amount: Amount) -> Result<Self, BulkPlanError> {
        if count.get() > BulkRequest::MAX_BATCH_COUNT {
            return Err(BulkPlanError::TooManyBatchTransactions(count.get()));
        }

        Self::check_amount(count, amount)?;
        Ok(Self(BulkShapeKind::Batch { count, amount }))
    }

    /// Creates one transaction spending all available inputs
    #[must_use]
    pub const fn consolidate() -> Self {
        Self(BulkShapeKind::Consolidate)
    }

    /// Returns the number of destination scripts needed
    #[must_use]
    pub const fn destination_count(&self) -> usize {
        match self.0 {
            BulkShapeKind::FanOut { count, .. } | BulkShapeKind::Batch { count, .. } => count.get(),
            BulkShapeKind::Consolidate => 1,
        }
    }

    /// Returns the operation kind
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        match self.0 {
            BulkShapeKind::FanOut { .. } => OperationKind::FanOut,
            BulkShapeKind::Batch { .. } => OperationKind::Batch,
            BulkShapeKind::Consolidate => OperationKind::Consolidate,
        }
    }

    fn check_amount(count: OutputCount, amount: Amount) -> Result<(), BulkPlanError> {
        if amount == Amount::ZERO {
            return Err(BulkPlanError::ZeroAmount);
        }

        amount
            .checked_mul(count.get() as u64)
            .ok_or(BulkPlanError::AmountOverflow)?;
        Ok(())
    }
}

/// A public destination or a saved signer resolved by the repository
#[derive(Debug, Clone)]
pub enum BulkDestinationSpec {
    /// Fresh receive addresses from another saved signer
    SavedSigner {
        /// Destination signer
        signer_id: SignerId,
        /// Destination account address type
        address_type: AddressType,
    },
    /// The same fixed address for every output
    Fixed(FixedAddress),
    /// Receive addresses from an account extended public key
    AccountXpub {
        /// Account extended public key
        xpub: Xpub,
        /// Destination address type
        address_type: AddressType,
        /// First non-hardened receive index
        start_index: u32,
    },
    /// Addresses from a ranged public descriptor
    Descriptor {
        /// Checked external descriptor
        descriptor: ExternalDescriptor,
        /// First non-hardened derivation index
        start_index: u32,
    },
}

impl BulkDestinationSpec {
    fn range(&self, count: usize) -> Result<Option<RangeInclusive<u32>>, DestinationError> {
        let start = match self {
            Self::AccountXpub { start_index, .. } | Self::Descriptor { start_index, .. } => {
                *start_index
            }
            Self::SavedSigner { .. } | Self::Fixed(_) => return Ok(None),
        };

        let count = u32::try_from(count).map_err(|_| DestinationError::IndexTooLarge)?;
        let end = start
            .checked_add(count)
            .ok_or(DestinationError::IndexTooLarge)?;

        if end > (1 << 31) {
            return Err(DestinationError::IndexTooLarge);
        }

        Ok(Some(start..=end - 1))
    }
}

/// A command to prepare a recoverable native `SegWit` bulk operation
#[derive(Debug, Clone)]
pub struct PlanBulk {
    /// Source signer
    pub signer: VirtualSigner,
    /// Explicit active network
    pub network: LabNetwork,
    /// Transaction layout
    pub shape: BulkShape,
    /// Destination source
    pub destination: BulkDestinationSpec,
    /// Requested miner fee rate
    pub fee_rate: LabFeeRate,
}

/// Input reservation outcome after the operation is saved
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputReservation {
    /// All planned inputs have persistent locks
    Held,
    /// The saved operation requires reservation recovery
    Failed(String),
}

/// A saved plan and its input reservation outcome
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBulkOperation {
    /// Saved operation identifier
    pub operation_id: OperationId,
    /// Planned transactions
    pub plan: BulkPlan,
    /// Result of locking the inputs after saving
    pub reservation: InputReservation,
    /// External derivation indexes, absent for fixed addresses and saved signers
    pub destination_range: Option<RangeInclusive<u32>>,
}

struct DestinationAllocation {
    wallet: NetworkWallet,
    address_type: AddressType,
    last_index: u32,
}

impl DestinationAllocation {
    fn persist(mut self) -> Result<(), WalletError> {
        let account = self.wallet.account_mut(self.address_type);
        let _ = account
            .bdk_mut()
            .reveal_addresses_to(bdk_wallet::KeychainKind::External, self.last_index);
        account.persist()?;
        Ok(())
    }
}

impl PlanBulk {
    fn validate(&self) -> Result<(), BulkPlanOperationError> {
        if let BulkDestinationSpec::SavedSigner { signer_id, .. } = &self.destination
            && *signer_id == self.signer.id
        {
            return Err(BulkPlanOperationError::SourceIsDestination);
        }

        Ok(())
    }
}

impl<S: SecretStore> SignerRepository<S> {
    /// Validates, resolves destinations, plans, saves, and then reserves inputs
    ///
    /// # Errors
    /// Returns typed validation, recovery, destination, wallet, or storage errors
    /// Destination persistence can fail after saving; the saved plan remains recoverable
    /// Reservation failure after saving is returned as an outcome, not an error
    pub async fn plan_bulk_operation(
        &mut self,
        request: PlanBulk,
    ) -> Result<PlannedBulkOperation, BulkPlanOperationError> {
        request.validate()?;
        let count = request.shape.destination_count();
        let destination_range = request.destination.range(count)?;

        self.reconcile_bulk_reservations(&request.signer, request.network)
            .await?;

        if let Some(CurrentBulkOperation::Active { operation_id, .. }) = self
            .current_bulk_operation(request.signer.id, request.network)
            .await?
        {
            return Err(RepositoryError::ActiveBulkOperation(operation_id).into());
        }

        let (scripts, allocation) = self
            .resolve_bulk_destination(&request.destination, request.network, count)
            .await?;

        let bulk_request = match request.shape.0 {
            BulkShapeKind::FanOut { amount, .. } => {
                BulkRequest::fan_out(amount, DestinationScripts::new(scripts)?)?
            }
            BulkShapeKind::Batch { amount, .. } => {
                BulkRequest::batch(amount, DestinationScripts::new(scripts)?)?
            }
            BulkShapeKind::Consolidate => BulkRequest::consolidate_all(
                scripts
                    .into_iter()
                    .next()
                    .ok_or(BulkPlanError::NoDestinations)?,
            ),
        };

        let mut wallet = self.open_wallet(&request.signer, request.network)?;
        let mut plan = BulkPlanner::new(wallet.account_mut(AddressType::NativeSegwit))
            .plan(&bulk_request, request.fee_rate)?;

        plan.set_destination_range(destination_range.clone());
        let operation = self
            .prepare_bulk_operation(request.signer.id, request.network, &plan)
            .await?;

        if let Some(allocation) = allocation {
            // a crash after saving but before revealing can let a later plan reuse these
            // still-unrevealed addresses, which is harmless because they are unused
            allocation.persist()?;
        }

        let reservation = match plan.reserve_inputs(wallet.account_mut(AddressType::NativeSegwit)) {
            Ok(()) => InputReservation::Held,
            Err(error) => InputReservation::Failed(error.to_string()),
        };

        Ok(PlannedBulkOperation {
            operation_id: operation.lifecycle.id,
            plan,
            reservation,
            destination_range,
        })
    }

    async fn resolve_bulk_destination(
        &mut self,
        destination: &BulkDestinationSpec,
        network: LabNetwork,
        count: usize,
    ) -> Result<(Vec<ScriptBuf>, Option<DestinationAllocation>), BulkPlanOperationError> {
        let (public, start_index) = match destination {
            BulkDestinationSpec::SavedSigner {
                signer_id,
                address_type,
            } => {
                let signer = self
                    .signers()
                    .await?
                    .into_iter()
                    .find(|signer| signer.id == *signer_id)
                    .ok_or(BulkPlanOperationError::DestinationSignerNotFound(
                        *signer_id,
                    ))?;

                let wallet = self.open_wallet(&signer, network)?;
                let account = wallet.account(*address_type).bdk();
                let start = account.next_derivation_index(bdk_wallet::KeychainKind::External);
                let count = u32::try_from(count).map_err(|_| DestinationError::IndexTooLarge)?;
                let end = start
                    .checked_add(count)
                    .filter(|end| *end <= (1 << 31))
                    .ok_or(DestinationError::IndexTooLarge)?;
                let scripts = (start..end)
                    .map(|index| {
                        account
                            .peek_address(bdk_wallet::KeychainKind::External, index)
                            .address
                            .script_pubkey()
                    })
                    .collect();

                return Ok((
                    scripts,
                    Some(DestinationAllocation {
                        wallet,
                        address_type: *address_type,
                        last_index: end - 1,
                    }),
                ));
            }
            BulkDestinationSpec::Fixed(address) => (Destination::Fixed(address.clone()), 0),
            BulkDestinationSpec::AccountXpub {
                xpub,
                address_type,
                start_index,
            } => (
                Destination::AccountXpub {
                    xpub: *xpub,
                    address_type: *address_type,
                },
                *start_index,
            ),
            BulkDestinationSpec::Descriptor {
                descriptor,
                start_index,
            } => (Destination::Descriptor(descriptor.clone()), *start_index),
        };

        Ok((public.derive_scripts(network, start_index, count)?, None))
    }
}

/// Errors during bulk planning or destination allocation
#[derive(Debug, thiserror::Error)]
pub enum BulkPlanOperationError {
    /// Source and destination must be distinct saved signers
    #[error("source signer cannot also be the destination")]
    SourceIsDestination,
    /// The destination signer is not saved
    #[error("destination signer {0} does not exist")]
    DestinationSignerNotFound(SignerId),
    /// Bulk request or planning failed
    #[error(transparent)]
    Plan(#[from] BulkPlanError),
    /// Destination validation or derivation failed
    #[error(transparent)]
    Destination(#[from] DestinationError),
    /// Repository storage or wallet access failed
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    /// Reservation recovery failed
    #[error(transparent)]
    Recovery(#[from] BulkExecutionError),
    /// Destination wallet persistence failed
    #[error(transparent)]
    Wallet(#[from] WalletError),
}
