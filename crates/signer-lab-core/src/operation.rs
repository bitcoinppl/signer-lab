use std::{fmt, str::FromStr};

use bitcoin::Txid;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stable identifier for a prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(Uuid);

impl OperationId {
    /// Creates a random operation identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for OperationId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// The transaction-building behavior of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// One transaction with many outputs.
    FanOut,
    /// Many independent transactions.
    Batch,
    /// Selected or all UTXOs sent back to the wallet.
    Consolidate,
}

/// The persisted state of an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OperationState {
    /// The transaction plan is stored and its inputs are reserved.
    Prepared,
    /// Transactions are being signed and broadcast.
    Running {
        /// Next transaction index to process.
        next_index: usize,
        /// Transactions already accepted for broadcast.
        accepted_txids: Vec<Txid>,
    },
    /// All transactions were processed.
    Completed {
        /// Accepted transaction identifiers.
        txids: Vec<Txid>,
    },
    /// No further transaction will be broadcast.
    Cancelled,
    /// Execution stopped after a failure.
    Failed {
        /// Index of the failed transaction.
        failed_index: usize,
        /// Accepted transactions before the failure.
        accepted_txids: Vec<Txid>,
        /// Stable user-visible failure text.
        error: String,
    },
    /// The application stopped during execution.
    Interrupted {
        /// Index that was active when the application stopped.
        next_index: usize,
        /// Transactions known to have been accepted.
        accepted_txids: Vec<Txid>,
    },
}

impl OperationState {
    /// Returns true when no more execution transitions are valid.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Completed { .. } | Self::Failed { .. }
        )
    }
}

/// Persisted operation state with transition ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationLifecycle {
    /// Operation identifier.
    pub id: OperationId,
    /// Operation behavior.
    pub kind: OperationKind,
    /// Current state.
    pub state: OperationState,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last transition timestamp.
    pub updated_at: Timestamp,
}

impl OperationLifecycle {
    /// Creates a prepared lifecycle.
    #[must_use]
    pub fn prepared(kind: OperationKind, now: Timestamp) -> Self {
        Self {
            id: OperationId::new(),
            kind,
            state: OperationState::Prepared,
            created_at: now,
            updated_at: now,
        }
    }

    /// Starts execution.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is prepared.
    pub fn start(&mut self, now: Timestamp) -> Result<(), StateTransitionError> {
        if self.state != OperationState::Prepared {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "start",
            });
        }

        self.state = OperationState::Running {
            next_index: 0,
            accepted_txids: Vec::new(),
        };
        self.updated_at = now;
        Ok(())
    }

    /// Records an accepted transaction and advances the next index.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is running at the supplied index.
    pub fn record_accepted(
        &mut self,
        index: usize,
        txid: Txid,
        now: Timestamp,
    ) -> Result<(), StateTransitionError> {
        let OperationState::Running {
            next_index,
            accepted_txids,
        } = &mut self.state
        else {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "record accepted transaction",
            });
        };

        if *next_index != index {
            return Err(StateTransitionError::UnexpectedIndex {
                expected: *next_index,
                actual: index,
            });
        }

        accepted_txids.push(txid);
        *next_index += 1;
        self.updated_at = now;
        Ok(())
    }

    /// Marks a running operation complete.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is running and all transactions ran.
    pub fn complete(
        &mut self,
        transaction_count: usize,
        now: Timestamp,
    ) -> Result<(), StateTransitionError> {
        let OperationState::Running {
            next_index,
            accepted_txids,
        } = &self.state
        else {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "complete",
            });
        };

        if *next_index != transaction_count {
            return Err(StateTransitionError::Incomplete {
                completed: *next_index,
                total: transaction_count,
            });
        }

        self.state = OperationState::Completed {
            txids: accepted_txids.clone(),
        };
        self.updated_at = now;
        Ok(())
    }

    /// Cancels a prepared operation.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is prepared.
    pub fn cancel(&mut self, now: Timestamp) -> Result<(), StateTransitionError> {
        if self.state != OperationState::Prepared {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "cancel",
            });
        }

        self.state = OperationState::Cancelled;
        self.updated_at = now;
        Ok(())
    }

    /// Stops a running operation after a transaction fails.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is running at the failed index.
    pub fn fail(
        &mut self,
        failed_index: usize,
        error: impl Into<String>,
        now: Timestamp,
    ) -> Result<(), StateTransitionError> {
        let OperationState::Running {
            next_index,
            accepted_txids,
        } = &self.state
        else {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "fail",
            });
        };

        if *next_index != failed_index {
            return Err(StateTransitionError::UnexpectedIndex {
                expected: *next_index,
                actual: failed_index,
            });
        }

        self.state = OperationState::Failed {
            failed_index,
            accepted_txids: accepted_txids.clone(),
            error: error.into(),
        };
        self.updated_at = now;
        Ok(())
    }

    /// Records that the application stopped during execution.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is running.
    pub fn interrupt(&mut self, now: Timestamp) -> Result<(), StateTransitionError> {
        let OperationState::Running {
            next_index,
            accepted_txids,
        } = &self.state
        else {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "interrupt",
            });
        };

        self.state = OperationState::Interrupted {
            next_index: *next_index,
            accepted_txids: accepted_txids.clone(),
        };
        self.updated_at = now;
        Ok(())
    }

    /// Resumes an operation from its persisted interrupted index.
    ///
    /// # Errors
    ///
    /// Returns an error unless the operation is interrupted.
    pub fn resume(&mut self, now: Timestamp) -> Result<(), StateTransitionError> {
        let OperationState::Interrupted {
            next_index,
            accepted_txids,
        } = &self.state
        else {
            return Err(StateTransitionError::Invalid {
                from: self.state.clone(),
                action: "resume",
            });
        };

        self.state = OperationState::Running {
            next_index: *next_index,
            accepted_txids: accepted_txids.clone(),
        };
        self.updated_at = now;
        Ok(())
    }
}

/// An invalid operation-state transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateTransitionError {
    /// The action is not valid from the current state.
    #[error("cannot {action} operation from state {from:?}")]
    Invalid {
        /// Current operation state.
        from: OperationState,
        /// Requested action.
        action: &'static str,
    },
    /// A transaction result arrived out of order.
    #[error("expected transaction index {expected}, got {actual}")]
    UnexpectedIndex {
        /// Expected transaction index.
        expected: usize,
        /// Actual transaction index.
        actual: usize,
    },
    /// Completion was requested before every transaction ran.
    #[error("operation processed {completed} of {total} transactions")]
    Incomplete {
        /// Completed transaction count.
        completed: usize,
        /// Planned transaction count.
        total: usize,
    },
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bitcoin::Txid;
    use jiff::Timestamp;

    use super::{OperationKind, OperationLifecycle, OperationState};

    #[test]
    fn lifecycle_only_completes_after_all_transactions() {
        let now = Timestamp::UNIX_EPOCH;
        let mut lifecycle = OperationLifecycle::prepared(OperationKind::Batch, now);
        lifecycle.start(now).expect("prepared lifecycle starts");

        assert!(lifecycle.complete(1, now).is_err());

        let txid =
            Txid::from_str("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("test txid is valid");
        lifecycle
            .record_accepted(0, txid, now)
            .expect("running lifecycle accepts the first transaction");
        lifecycle
            .complete(1, now)
            .expect("all planned transactions completed");

        assert!(matches!(lifecycle.state, OperationState::Completed { .. }));
    }

    #[test]
    fn interrupted_operation_resumes_at_the_same_index() {
        let now = Timestamp::UNIX_EPOCH;
        let mut lifecycle = OperationLifecycle::prepared(OperationKind::Batch, now);
        lifecycle.start(now).expect("prepared lifecycle starts");
        let txid =
            Txid::from_str("0000000000000000000000000000000000000000000000000000000000000001")
                .expect("test txid is valid");
        lifecycle
            .record_accepted(0, txid, now)
            .expect("running lifecycle accepts the first transaction");
        lifecycle
            .interrupt(now)
            .expect("running lifecycle can be interrupted");
        lifecycle
            .resume(now)
            .expect("interrupted lifecycle can resume");

        assert_eq!(
            lifecycle.state,
            OperationState::Running {
                next_index: 1,
                accepted_txids: vec![txid]
            }
        );
    }
}
