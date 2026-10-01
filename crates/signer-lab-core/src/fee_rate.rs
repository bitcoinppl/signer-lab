use std::{fmt, str::FromStr};

use bitcoin::FeeRate;
use serde::{Deserialize, Serialize};

/// An exact fee rate with a version 1 minimum of 0.1 sat/vB
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "UncheckedFeeRate")]
pub struct LabFeeRate {
    sat_per_kwu: u64,
}

#[derive(Deserialize)]
struct UncheckedFeeRate {
    sat_per_kwu: u64,
}

impl TryFrom<UncheckedFeeRate> for LabFeeRate {
    type Error = FeeRateError;

    fn try_from(value: UncheckedFeeRate) -> Result<Self, Self::Error> {
        Self::from_sat_per_kwu(value.sat_per_kwu)
    }
}

impl LabFeeRate {
    /// Minimum fee rate in satoshis per 1,000 weight units
    pub const MIN_SAT_PER_KWU: u64 = 25;

    /// Maximum fee rate of 100,000 sat/vB
    pub const MAX_SAT_PER_KWU: u64 = 25_000_000;

    /// Default fee rate of 0.5 sat/vB
    pub const DEFAULT: Self = Self { sat_per_kwu: 125 };

    /// Creates a fee rate from satoshis per 1,000 weight units
    ///
    /// # Errors
    ///
    /// Returns an error when the rate is outside 0.1 through 100,000 sat/vB
    pub const fn from_sat_per_kwu(sat_per_kwu: u64) -> Result<Self, FeeRateError> {
        if sat_per_kwu < Self::MIN_SAT_PER_KWU {
            return Err(FeeRateError::BelowMinimum);
        }

        if sat_per_kwu > Self::MAX_SAT_PER_KWU {
            return Err(FeeRateError::AboveMaximum);
        }

        Ok(Self { sat_per_kwu })
    }

    /// Returns the BDK fee-rate value
    #[must_use]
    pub const fn as_bitcoin(self) -> FeeRate {
        FeeRate::from_sat_per_kwu(self.sat_per_kwu)
    }

    /// Returns satoshis per 1,000 weight units
    #[must_use]
    pub const fn to_sat_per_kwu(self) -> u64 {
        self.sat_per_kwu
    }
}

impl Default for LabFeeRate {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl FromStr for LabFeeRate {
    type Err = FeeRateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() || value.starts_with('-') {
            return Err(FeeRateError::InvalidNumber);
        }

        let mut parts = value.split('.');
        let whole = parts.next().ok_or(FeeRateError::InvalidNumber)?;
        let fraction = parts.next();
        if parts.next().is_some() {
            return Err(FeeRateError::InvalidNumber);
        }

        let whole: u64 = whole.parse().map_err(|_| FeeRateError::InvalidNumber)?;
        let fraction = fraction.unwrap_or("");
        if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(FeeRateError::InvalidPrecision);
        }

        let padded = format!("{fraction:0<3}");
        let milli_fraction: u64 = padded.parse().map_err(|_| FeeRateError::InvalidNumber)?;
        let milli_sat_per_vbyte = whole
            .checked_mul(1_000)
            .and_then(|value| value.checked_add(milli_fraction))
            .ok_or(FeeRateError::Overflow)?;

        if milli_sat_per_vbyte < 100 {
            return Err(FeeRateError::BelowMinimum);
        }

        // one sat/kwu is 0.004 sat/vB, so round user input upward to avoid underpaying
        let sat_per_kwu = milli_sat_per_vbyte
            .checked_add(3)
            .ok_or(FeeRateError::Overflow)?
            / 4;

        Self::from_sat_per_kwu(sat_per_kwu)
    }
}

impl fmt::Display for LabFeeRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let milli_sat_per_vbyte = self.sat_per_kwu * 4;
        let whole = milli_sat_per_vbyte / 1_000;
        let fraction = milli_sat_per_vbyte % 1_000;

        if fraction == 0 {
            return write!(formatter, "{whole}");
        }

        let mut display_value = format!("{whole}.{fraction:03}");
        while display_value.ends_with('0') {
            display_value.pop();
        }

        formatter.write_str(&display_value)
    }
}

/// Errors from parsing or validating a fee rate
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FeeRateError {
    /// The text is not a positive decimal number
    #[error("fee rate must be a positive decimal number")]
    InvalidNumber,
    /// The text has more than three decimal places
    #[error("fee rate supports at most three decimal places")]
    InvalidPrecision,
    /// The rate is lower than 0.1 sat/vB
    #[error("fee rate must be at least 0.1 sat/vB")]
    BelowMinimum,
    /// The rate exceeds 100,000 sat/vB
    #[error("fee rate must not exceed 100000 sat/vB")]
    AboveMaximum,
    /// The value cannot be represented
    #[error("fee rate is too large")]
    Overflow,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::LabFeeRate;

    #[test]
    fn fractional_fee_rates_use_exact_weight_units() {
        let rate = LabFeeRate::from_str("0.1").expect("valid fractional fee rate");

        assert_eq!(rate.to_sat_per_kwu(), 25);
        assert_eq!(rate.to_string(), "0.1");
    }

    #[test]
    fn fractional_rate_rounds_up_to_next_weight_unit() {
        let rate = LabFeeRate::from_str("0.101").expect("valid fractional fee rate");

        assert_eq!(rate.to_sat_per_kwu(), 26);
        assert_eq!(rate.to_string(), "0.104");
    }

    #[test]
    fn fee_rate_rejects_values_below_version_one_minimum() {
        assert!(LabFeeRate::from_str("0.099").is_err());
    }

    #[test]
    fn maximum_rate_is_bounded_and_safe_for_block_weight() {
        let maximum = LabFeeRate::MAX_SAT_PER_KWU;
        assert_eq!(
            LabFeeRate::from_sat_per_kwu(maximum + 1),
            Err(super::FeeRateError::AboveMaximum)
        );

        assert_eq!(
            LabFeeRate::from_str("100000.004"),
            Err(super::FeeRateError::AboveMaximum)
        );

        assert!(
            serde_json::from_str::<LabFeeRate>(&format!("{{\"sat_per_kwu\":{}}}", maximum + 1))
                .is_err()
        );

        let rate = LabFeeRate::from_sat_per_kwu(maximum).expect("maximum rate");
        assert!(
            rate.as_bitcoin()
                .fee_wu(bitcoin::Weight::MAX_BLOCK)
                .is_some()
        );

        assert_eq!(rate.to_string(), "100000");
    }

    #[test]
    fn saved_fee_rate_cannot_bypass_minimum() {
        assert!(serde_json::from_str::<LabFeeRate>(r#"{"sat_per_kwu":0}"#).is_err());
        let json = serde_json::to_string(&LabFeeRate::DEFAULT).expect("rate serializes");
        assert_eq!(
            serde_json::from_str::<LabFeeRate>(&json).expect("rate decodes"),
            LabFeeRate::DEFAULT
        );
    }
}
