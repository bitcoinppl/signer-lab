use std::fmt;

use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

use crate::{MnemonicError, MnemonicSecret};

const INDEX_DIGITS: usize = 4;
const MAX_WORD_INDEX: u16 = 2047;
const DECIMAL_DIGITS: [char; 10] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

/// A `SeedQR` payload whose encoding is explicit in the type
pub enum SeedQr {
    /// Zero-padded four-digit English BIP39 word indexes
    Standard(StandardSeedQr),
    /// Raw BIP39 entropy bytes
    Compact(CompactSeedQr),
}

impl SeedQr {
    /// Returns the validated mnemonic secret carried by this `SeedQR`
    #[must_use]
    pub fn into_secret(self) -> MnemonicSecret {
        match self {
            Self::Standard(seed_qr) => seed_qr.into_secret(),
            Self::Compact(seed_qr) => seed_qr.into_secret(),
        }
    }
}

impl fmt::Debug for SeedQr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standard(_) => formatter.write_str("SeedQr::Standard([REDACTED])"),
            Self::Compact(_) => formatter.write_str("SeedQr::Compact([REDACTED])"),
        }
    }
}

/// A validated standard `SeedQR` backed by its English BIP39 mnemonic
pub struct StandardSeedQr(MnemonicSecret);

impl StandardSeedQr {
    /// Parses standard `SeedQR` digits and validates the reconstructed BIP39 mnemonic
    ///
    /// # Errors
    ///
    /// Returns an error for nonnumeric input, invalid indexes, unsupported word counts, or a bad
    /// BIP39 checksum
    pub fn parse(value: &str) -> Result<Self, SeedQrError> {
        let indexes = word_indexes(value.trim())?;
        let words = indexes
            .into_iter()
            .map(|index| Language::English.word_list()[usize::from(index)])
            .collect::<Vec<_>>()
            .join(" ");

        let secret = MnemonicSecret::parse(&words).map_err(SeedQrError::InvalidSecret)?;

        Ok(Self(secret))
    }

    /// Builds a standard `SeedQR` from a checked mnemonic secret
    ///
    /// # Errors
    ///
    /// Returns an error only if the protected mnemonic was corrupted after construction
    pub fn from_secret(secret: &MnemonicSecret) -> Result<Self, SeedQrError> {
        let secret =
            MnemonicSecret::parse(secret.expose_phrase()).map_err(SeedQrError::InvalidSecret)?;

        Ok(Self(secret))
    }

    /// Encodes the standard `SeedQR` digits in zeroizing memory
    ///
    /// # Errors
    ///
    /// Returns an error only if protected mnemonic data was corrupted after construction
    pub fn numeric_text(&self) -> Result<Zeroizing<String>, SeedQrError> {
        let mnemonic = self.0.to_mnemonic().map_err(SeedQrError::InvalidSecret)?;
        let word_list = Language::English.word_list();
        let mut text = Zeroizing::new(String::with_capacity(mnemonic.word_count() * INDEX_DIGITS));
        for word in mnemonic.words() {
            let index = word_list
                .iter()
                .position(|candidate| *candidate == word)
                .ok_or_else(|| SeedQrError::UnknownWord(word.to_owned()))?;

            for divisor in [1000, 100, 10, 1] {
                text.push(DECIMAL_DIGITS[(index / divisor) % 10]);
            }
        }

        Ok(text)
    }

    /// Returns the validated mnemonic secret carried by this `SeedQR`
    #[must_use]
    pub fn into_secret(self) -> MnemonicSecret {
        self.0
    }
}

impl fmt::Debug for StandardSeedQr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StandardSeedQr([REDACTED])")
    }
}

/// A validated compact `SeedQR` backed by its English BIP39 mnemonic
pub struct CompactSeedQr(MnemonicSecret);

impl CompactSeedQr {
    /// Parses compact `SeedQR` entropy
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not a supported BIP39 entropy length
    pub fn parse(entropy: &[u8]) -> Result<Self, SeedQrError> {
        let mnemonic = Mnemonic::from_entropy_in(Language::English, entropy)
            .map_err(SeedQrError::InvalidMnemonic)?;

        let secret =
            MnemonicSecret::parse(&mnemonic.to_string()).map_err(SeedQrError::InvalidSecret)?;

        Ok(Self(secret))
    }

    /// Builds a compact `SeedQR` from a checked mnemonic secret
    ///
    /// # Errors
    ///
    /// Returns an error only if the protected mnemonic was corrupted after construction
    pub fn from_secret(secret: &MnemonicSecret) -> Result<Self, SeedQrError> {
        let secret =
            MnemonicSecret::parse(secret.expose_phrase()).map_err(SeedQrError::InvalidSecret)?;

        Ok(Self(secret))
    }

    /// Returns raw BIP39 entropy in zeroizing memory
    ///
    /// # Errors
    ///
    /// Returns an error only if protected mnemonic data was corrupted after construction
    pub fn entropy(&self) -> Result<Zeroizing<Vec<u8>>, SeedQrError> {
        self.0
            .to_mnemonic()
            .map(|mnemonic| Zeroizing::new(mnemonic.to_entropy()))
            .map_err(SeedQrError::InvalidSecret)
    }

    /// Returns the validated mnemonic secret carried by this `SeedQR`
    #[must_use]
    pub fn into_secret(self) -> MnemonicSecret {
        self.0
    }
}

impl fmt::Debug for CompactSeedQr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CompactSeedQr([REDACTED])")
    }
}

fn word_indexes(value: &str) -> Result<Vec<u16>, SeedQrError> {
    if !value.is_ascii() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SeedQrError::NonNumeric);
    }

    if !value.len().is_multiple_of(INDEX_DIGITS) {
        return Err(SeedQrError::InvalidLength(value.len()));
    }

    let word_count = value.len() / INDEX_DIGITS;
    if !matches!(word_count, 12 | 15 | 18 | 21 | 24) {
        return Err(SeedQrError::UnsupportedWordCount(word_count));
    }

    value
        .as_bytes()
        .as_chunks::<INDEX_DIGITS>()
        .0
        .iter()
        .map(|digits| {
            let index = u16::from(digits[0] - b'0') * 1000
                + u16::from(digits[1] - b'0') * 100
                + u16::from(digits[2] - b'0') * 10
                + u16::from(digits[3] - b'0');

            if index > MAX_WORD_INDEX {
                return Err(SeedQrError::IndexOutOfBounds(index));
            }

            Ok(index)
        })
        .collect()
}

/// Errors from standard or compact `SeedQR` conversion
#[derive(Debug, thiserror::Error)]
pub enum SeedQrError {
    /// Standard `SeedQR` accepts only ASCII digits
    #[error("standard SeedQR must contain only ASCII digits")]
    NonNumeric,
    /// Standard `SeedQR` must contain complete four-digit indexes
    #[error("standard SeedQR length must be a multiple of four, got {0}")]
    InvalidLength(usize),
    /// A BIP39 word index was larger than 2047
    #[error("SeedQR word index {0} is larger than 2047")]
    IndexOutOfBounds(u16),
    /// `SeedQR` supports the standard BIP39 word counts
    #[error("SeedQR word count must be 12, 15, 18, 21, or 24, got {0}")]
    UnsupportedWordCount(usize),
    /// A mnemonic word was not in the English BIP39 list
    #[error("SeedQR cannot encode unknown BIP39 word: {0}")]
    UnknownWord(String),
    /// Compact entropy was not valid BIP39 input
    #[error("compact SeedQR contains invalid BIP39 entropy: {0}")]
    InvalidMnemonic(#[source] bip39::Error),
    /// Protected mnemonic data was invalid after construction
    #[error("SeedQR secret conversion failed: {0}")]
    InvalidSecret(#[source] MnemonicError),
}

#[cfg(test)]
mod tests {
    use super::{CompactSeedQr, SeedQrError, StandardSeedQr};
    use crate::MnemonicSecret;

    const WORDS: &str = "forum undo fragile fade shy sign arrest garment culture tube off merit";
    const STANDARD: &str = "073318950739065415961602009907670428187212261116";
    const COMPACT: [u8; 16] = [
        0x5b, 0xbd, 0x9d, 0x71, 0xa8, 0xec, 0x79, 0x90, 0x83, 0x1a, 0xff, 0x35, 0x9d, 0x42, 0x65,
        0x45,
    ];

    #[test]
    fn standard_and_compact_seed_qr_match_cove_vector() {
        let standard = StandardSeedQr::parse(STANDARD).expect("standard SeedQR is valid");
        let compact = CompactSeedQr::parse(&COMPACT).expect("compact SeedQR is valid");

        assert_eq!(
            standard.numeric_text().expect("standard text").as_str(),
            STANDARD
        );

        assert_eq!(
            compact.entropy().expect("compact entropy").as_slice(),
            COMPACT
        );

        assert_eq!(standard.into_secret().expose_phrase(), WORDS);
        assert_eq!(compact.into_secret().expose_phrase(), WORDS);
    }

    #[test]
    fn mnemonic_exports_with_leading_zeroes() {
        let secret = MnemonicSecret::parse(WORDS).expect("test mnemonic is valid");
        let seed_qr = StandardSeedQr::from_secret(&secret).expect("mnemonic exports as SeedQR");

        assert_eq!(
            seed_qr.numeric_text().expect("standard text").as_str(),
            STANDARD
        );
    }

    #[test]
    fn invalid_standard_seed_qr_is_rejected_without_exposing_secret_debug() {
        assert!(matches!(
            StandardSeedQr::parse("0000not-digits"),
            Err(SeedQrError::NonNumeric)
        ));

        assert!(matches!(
            StandardSeedQr::parse(&"2048".repeat(12)),
            Err(SeedQrError::IndexOutOfBounds(2048))
        ));

        let seed_qr = StandardSeedQr::parse(STANDARD).expect("standard SeedQR is valid");
        assert_eq!(format!("{seed_qr:?}"), "StandardSeedQr([REDACTED])");
    }
}
