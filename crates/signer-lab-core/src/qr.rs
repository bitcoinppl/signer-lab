use std::collections::HashSet;

use bbqr::{
    continuous_join::{ContinuousJoinResult, ContinuousJoiner},
    encode::Encoding,
    file_type::FileType,
    header::Header,
    qr::Version,
    split::{Split, SplitOptions},
};
use bitcoin::Psbt;
use cove_ur::CryptoPsbt;
use cove_util::ResultExt as _;
use foundation_ur::{Decoder as UrDecoder, Encoder as UrEncoder, UR};
use serde::{Deserialize, Serialize};

/// A supported animated QR transport
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrTransport {
    /// Binary Bitcoin QR
    #[default]
    Bbqr,
    /// Uniform Resources `crypto-psbt`
    Ur,
}

/// QR density controls for both transports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedDensity")]
pub struct QrDensity {
    ur_fragment_len: usize,
    bbqr_version: u8,
}

#[derive(Deserialize)]
struct UncheckedDensity {
    ur_fragment_len: usize,
    bbqr_version: u8,
}

impl TryFrom<UncheckedDensity> for QrDensity {
    type Error = QrTransportError;

    fn try_from(value: UncheckedDensity) -> Result<Self, Self::Error> {
        Self::new(value.ur_fragment_len, value.bbqr_version)
    }
}

impl QrDensity {
    /// Creates checked QR density settings
    ///
    /// # Errors
    ///
    /// Returns an error when UR length is outside 25 through 500 or BBQR version is outside 5 through 40
    pub const fn new(ur_fragment_len: usize, bbqr_version: u8) -> Result<Self, QrTransportError> {
        if ur_fragment_len < 25 || ur_fragment_len > 500 {
            return Err(QrTransportError::InvalidDensity);
        }

        if bbqr_version < 5 || bbqr_version > 40 {
            return Err(QrTransportError::InvalidDensity);
        }

        Ok(Self {
            ur_fragment_len,
            bbqr_version,
        })
    }

    /// Returns the recommended animation interval
    #[must_use]
    pub const fn animation_interval_ms(self, transport: QrTransport) -> u64 {
        match transport {
            QrTransport::Bbqr if self.bbqr_version >= 15 => 250,
            QrTransport::Bbqr if self.bbqr_version >= 9 => 350,
            QrTransport::Ur if self.ur_fragment_len >= 200 => 250,
            QrTransport::Ur if self.ur_fragment_len >= 100 => 350,
            QrTransport::Bbqr | QrTransport::Ur => 450,
        }
    }
}

impl Default for QrDensity {
    fn default() -> Self {
        Self {
            ur_fragment_len: 200,
            bbqr_version: 15,
        }
    }
}

/// Frames ready for animated display
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrFrames {
    /// Transport used to encode the frames
    pub transport: QrTransport,
    /// Encoded frame strings
    pub frames: Vec<String>,
    /// Recommended frame interval
    pub interval_ms: u64,
}

impl QrFrames {
    /// Encodes a PSBT with the selected transport
    ///
    /// # Errors
    ///
    /// Returns an error when the PSBT cannot fit or the UR payload cannot be encoded
    pub fn from_psbt(
        psbt: &Psbt,
        transport: QrTransport,
        density: QrDensity,
    ) -> Result<Self, QrTransportError> {
        let frames = match transport {
            QrTransport::Bbqr => encode_bbqr_data(&psbt.serialize(), FileType::Psbt, density, 1)?,
            QrTransport::Ur => encode_ur(psbt, density)?,
        };

        Ok(Self {
            transport,
            frames,
            interval_ms: density.animation_interval_ms(transport),
        })
    }

    pub(crate) fn from_bbqr_data(
        data: &[u8],
        file_type: FileType,
        density: QrDensity,
    ) -> Result<Self, QrTransportError> {
        let frames = encode_bbqr_data(data, file_type, density, 2)?;

        Ok(Self {
            transport: QrTransport::Bbqr,
            frames,
            interval_ms: density.animation_interval_ms(QrTransport::Bbqr),
        })
    }

    pub(crate) fn from_ur_data(ur_type: &str, cbor: &[u8], density: QrDensity) -> Self {
        let frames = encode_ur_data(ur_type, cbor, density.ur_fragment_len);

        Self {
            transport: QrTransport::Ur,
            frames,
            interval_ms: density.animation_interval_ms(QrTransport::Ur),
        }
    }
}

/// Progress or completion from one scanned QR frame
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrScanProgress {
    /// More frames are required
    InProgress {
        /// Transport detected from the first frame
        transport: QrTransport,
        /// Approximate completion percentage from 0 through 100
        percent: u8,
    },
    /// A complete PSBT was decoded
    Complete {
        /// Input transport
        transport: QrTransport,
        /// Decoded PSBT
        psbt: Psbt,
    },
}

enum ScannerMode {
    Empty,
    Bbqr {
        joiner: ContinuousJoiner,
        seen: HashSet<String>,
        total: usize,
    },
    Ur {
        decoder: Box<UrDecoder>,
        seen: HashSet<String>,
    },
}

/// Stateful multipart BBQR and UR scanner
pub struct QrScanner {
    mode: ScannerMode,
}

impl QrScanner {
    /// Creates an empty scanner
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: ScannerMode::Empty,
        }
    }

    /// Clears all multipart state
    pub fn reset(&mut self) {
        self.mode = ScannerMode::Empty;
    }

    /// Adds one decoded QR string
    ///
    /// # Errors
    ///
    /// Returns an error for invalid data, mixed transports, non-PSBT BBQR data, or non-PSBT UR data
    pub fn receive(&mut self, frame: &str) -> Result<QrScanProgress, QrTransportError> {
        let frame = frame.trim();
        if frame.starts_with("B$") {
            return self.receive_bbqr(frame);
        }

        let normalized = frame.to_ascii_lowercase();
        if normalized.starts_with("ur:") {
            return self.receive_ur(&normalized);
        }

        Err(QrTransportError::UnknownTransport)
    }

    fn receive_bbqr(&mut self, frame: &str) -> Result<QrScanProgress, QrTransportError> {
        if matches!(self.mode, ScannerMode::Empty) {
            let header = Header::try_from_str(frame).map_err_str(QrTransportError::Bbqr)?;
            self.mode = ScannerMode::Bbqr {
                joiner: ContinuousJoiner::new(),
                seen: HashSet::new(),
                total: header.num_parts,
            };
        }

        let ScannerMode::Bbqr {
            joiner,
            seen,
            total,
        } = &mut self.mode
        else {
            return Err(QrTransportError::MixedTransports);
        };

        let result = joiner
            .add_part(frame.to_owned())
            .map_err_str(QrTransportError::Bbqr)?;

        seen.insert(frame.to_owned());

        if let ContinuousJoinResult::Complete(joined) = result {
            if joined.file_type != FileType::Psbt {
                return Err(QrTransportError::WrongPayload);
            }

            let psbt = Psbt::deserialize(&joined.data).map_err(QrTransportError::Psbt)?;
            return Ok(QrScanProgress::Complete {
                transport: QrTransport::Bbqr,
                psbt,
            });
        }

        let percent = percent(seen.len(), *total);
        Ok(QrScanProgress::InProgress {
            transport: QrTransport::Bbqr,
            percent,
        })
    }

    fn receive_ur(&mut self, frame: &str) -> Result<QrScanProgress, QrTransportError> {
        if matches!(self.mode, ScannerMode::Bbqr { .. }) {
            return Err(QrTransportError::MixedTransports);
        }

        let ur = UR::parse(frame).map_err_str(QrTransportError::Ur)?;
        if ur.as_type() != "crypto-psbt" {
            return Err(QrTransportError::WrongPayload);
        }

        if matches!(self.mode, ScannerMode::Empty) {
            self.mode = ScannerMode::Ur {
                decoder: Box::new(UrDecoder::default()),
                seen: HashSet::new(),
            };
        }

        if !ur.is_multi_part() {
            let crypto_psbt =
                CryptoPsbt::from_ur_string(frame).map_err_str(QrTransportError::Ur)?;

            return Ok(QrScanProgress::Complete {
                transport: QrTransport::Ur,
                psbt: crypto_psbt.psbt().clone(),
            });
        }

        let ScannerMode::Ur { decoder, seen } = &mut self.mode else {
            return Err(QrTransportError::MixedTransports);
        };

        if !seen.contains(frame) {
            decoder.receive(ur).map_err_str(QrTransportError::Ur)?;
            seen.insert(frame.to_owned());
        }

        if decoder.is_complete() {
            let message = decoder
                .message()
                .map_err_str(QrTransportError::Ur)?
                .ok_or_else(|| {
                    QrTransportError::Ur("complete decoder has no message".to_owned())
                })?;

            let crypto_psbt = CryptoPsbt::from_cbor(message).map_err_str(QrTransportError::Ur)?;

            return Ok(QrScanProgress::Complete {
                transport: QrTransport::Ur,
                psbt: crypto_psbt.psbt().clone(),
            });
        }

        let percent = (decoder.estimated_percent_complete() * 100.0)
            .clamp(0.0, 99.0)
            .floor()
            .to_string()
            .parse()
            .unwrap_or_default();
        Ok(QrScanProgress::InProgress {
            transport: QrTransport::Ur,
            percent,
        })
    }
}

impl Default for QrScanner {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_bbqr_data(
    data: &[u8],
    file_type: FileType,
    density: QrDensity,
    minimum_frame_count: usize,
) -> Result<Vec<String>, QrTransportError> {
    let split = Split::try_from_data(
        data,
        file_type,
        SplitOptions {
            encoding: Encoding::Zlib,
            min_split_number: minimum_frame_count,
            max_split_number: 100,
            min_version: Version::V01,
            max_version: Version::try_from(density.bbqr_version)
                .map_err(|_| QrTransportError::InvalidDensity)?,
        },
    )
    .map_err_str(QrTransportError::Bbqr)?;

    Ok(split.parts)
}

fn encode_ur(psbt: &Psbt, density: QrDensity) -> Result<Vec<String>, QrTransportError> {
    let cbor = CryptoPsbt::new(psbt.clone())
        .to_cbor()
        .map_err_str(QrTransportError::Ur)?;
    Ok(encode_ur_data(
        "crypto-psbt",
        &cbor,
        density.ur_fragment_len,
    ))
}

pub(crate) fn encode_ur_data(ur_type: &str, cbor: &[u8], fragment_len: usize) -> Vec<String> {
    let mut encoder = UrEncoder::new();
    encoder.start(ur_type, cbor, fragment_len);
    let frame_count = encoder.sequence_count() as usize;

    (0..frame_count)
        .map(|_| encoder.next_part().to_string())
        .collect()
}

fn percent(received: usize, total: usize) -> u8 {
    if total == 0 {
        return 0;
    }

    u8::try_from((received.saturating_mul(100) / total).min(99)).unwrap_or_default()
}

/// QR framing and parsing errors
#[derive(Debug, thiserror::Error)]
pub enum QrTransportError {
    /// QR density settings are outside supported limits
    #[error("invalid QR density")]
    InvalidDensity,
    /// The frame is not BBQR or UR
    #[error("QR data is not BBQR or UR")]
    UnknownTransport,
    /// One scan contains both BBQR and UR frames
    #[error("cannot mix BBQR and UR frames in one scan")]
    MixedTransports,
    /// The QR transport contains a non-PSBT payload
    #[error("QR payload is not a PSBT")]
    WrongPayload,
    /// BBQR framing failed
    #[error("BBQR operation failed: {0}")]
    Bbqr(String),
    /// UR framing failed
    #[error("UR operation failed: {0}")]
    Ur(String),
    /// PSBT data is invalid
    #[error("invalid PSBT: {0}")]
    Psbt(#[source] bitcoin::psbt::Error),
}

#[cfg(test)]
mod tests {
    use bitcoin::{Psbt, Transaction, absolute::LockTime, transaction::Version};

    use super::{QrDensity, QrFrames, QrScanProgress, QrScanner, QrTransport};

    fn empty_psbt() -> Psbt {
        Psbt::from_unsigned_tx(Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        })
        .expect("empty transaction is a valid PSBT")
    }

    #[test]
    fn transports_round_trip_cove_crypto_psbt() {
        for transport in [QrTransport::Bbqr, QrTransport::Ur] {
            let expected = empty_psbt();
            let frames = QrFrames::from_psbt(&expected, transport, QrDensity::default())
                .expect("empty PSBT encodes for every supported transport");

            let mut scanner = QrScanner::new();
            let mut complete = None;

            for frame in frames.frames.into_iter().rev() {
                if let QrScanProgress::Complete { psbt, .. } = scanner
                    .receive(&frame)
                    .expect("encoded frame decodes successfully")
                {
                    complete = Some(psbt);
                    break;
                }
            }

            assert_eq!(complete, Some(expected));
        }
    }
}
