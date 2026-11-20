use std::{fs, path::Path, str::FromStr as _};

use bitcoin::Psbt;

use crate::{
    ExportArtifact, ExportPayload, QrDensity, QrScanProgress, QrScanner, QrTransport,
    qr::psbt_text_frames,
};

const PSBT_MAGIC: &[u8] = b"psbt\xff";

/// A supported PSBT file representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsbtFileFormat {
    /// Base64 PSBT text
    Base64,
    /// Binary PSBT data
    Binary,
    /// One BBQR frame per line
    Bbqr,
    /// One `crypto-psbt` UR frame per line
    Ur,
}

/// Progress from PSBT text or multipart QR decoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsbtDecodeProgress {
    /// A complete PSBT was decoded
    Complete(Psbt),
    /// More QR frames are required
    InProgress {
        /// Approximate completion percentage from 0 through 100
        percent: u8,
    },
}

/// Decodes a binary, Base64, BBQR, or UR PSBT document
///
/// # Errors
///
/// Returns an error for invalid text, PSBT data, QR frames, or incomplete multipart input
pub fn decode_psbt(bytes: &[u8]) -> Result<Psbt, PsbtFileError> {
    if bytes.starts_with(PSBT_MAGIC) {
        return Psbt::deserialize(bytes).map_err(PsbtFileError::Binary);
    }

    let text = std::str::from_utf8(bytes).map_err(PsbtFileError::Utf8)?;
    let mut scanner = QrScanner::new();
    match decode_psbt_text(text, &mut scanner)? {
        PsbtDecodeProgress::Complete(psbt) => Ok(psbt),
        PsbtDecodeProgress::InProgress { .. } => Err(PsbtFileError::IncompleteQr),
    }
}

/// Decodes Base64 PSBT text or advances a multipart BBQR or UR scan
///
/// # Errors
///
/// Returns an error for invalid PSBT text or QR frames
pub fn decode_psbt_text(
    text: &str,
    scanner: &mut QrScanner,
) -> Result<PsbtDecodeProgress, PsbtFileError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PsbtFileError::Empty);
    }

    if !text.starts_with("B$") && !text.to_ascii_lowercase().starts_with("ur:") {
        return Psbt::from_str(text)
            .map(PsbtDecodeProgress::Complete)
            .map_err(PsbtFileError::Base64);
    }

    let mut percent = 0;
    for frame in text
        .lines()
        .map(str::trim)
        .filter(|frame| !frame.is_empty())
    {
        match scanner.receive(frame).map_err(PsbtFileError::Qr)? {
            QrScanProgress::InProgress {
                percent: frame_percent,
                ..
            } => percent = frame_percent,
            QrScanProgress::Complete { psbt, .. } => {
                return Ok(PsbtDecodeProgress::Complete(psbt));
            }
        }
    }

    Ok(PsbtDecodeProgress::InProgress { percent })
}

/// Reads and decodes a PSBT document from a file
///
/// # Errors
///
/// Returns an error when the file cannot be read or its PSBT data is invalid
pub fn read_psbt(path: impl AsRef<Path>) -> Result<Psbt, PsbtFileError> {
    let bytes = fs::read(path).map_err(PsbtFileError::Io)?;

    decode_psbt(&bytes)
}

/// Writes a PSBT in one selected file representation
///
/// # Errors
///
/// Returns an error when QR framing or file writing fails
pub fn write_psbt(
    path: impl AsRef<Path>,
    psbt: &Psbt,
    format: PsbtFileFormat,
) -> Result<(), PsbtFileError> {
    let bytes = match format {
        PsbtFileFormat::Base64 => psbt.to_string().into_bytes(),
        PsbtFileFormat::Binary => psbt.serialize(),
        PsbtFileFormat::Bbqr => qr_document(psbt, QrTransport::Bbqr)?.into_bytes(),
        PsbtFileFormat::Ur => qr_document(psbt, QrTransport::Ur)?.into_bytes(),
    };

    fs::write(path, bytes).map_err(PsbtFileError::Io)
}

impl ExportArtifact {
    /// Writes this wallet export to a file
    ///
    /// Text payloads retain their text form. Multipart UR payloads use one frame per line
    ///
    /// # Errors
    ///
    /// Returns an error when the destination cannot be written
    pub fn write_to_path(&self, path: impl AsRef<Path>) -> Result<(), ArtifactFileError> {
        let document = match &self.payload {
            ExportPayload::Text(text) => text.clone(),
            ExportPayload::Ur { frames, .. } => frames.join("\n"),
        };

        fs::write(path, document).map_err(ArtifactFileError::Io)
    }
}

fn qr_document(psbt: &Psbt, transport: QrTransport) -> Result<String, PsbtFileError> {
    let frames = psbt_text_frames(psbt, transport, QrDensity::default())?;

    Ok(frames.join("\n"))
}

/// Wallet-export file errors
#[derive(Debug, thiserror::Error)]
pub enum ArtifactFileError {
    /// The destination could not be written
    #[error("wallet export file operation failed: {0}")]
    Io(#[source] std::io::Error),
}

/// PSBT file decoding and writing errors
#[derive(Debug, thiserror::Error)]
pub enum PsbtFileError {
    /// The input contains no PSBT data
    #[error("PSBT input is empty")]
    Empty,
    /// The source could not be read or the destination could not be written
    #[error("PSBT file operation failed: {0}")]
    Io(#[source] std::io::Error),
    /// Text input is not UTF-8
    #[error("PSBT text is not UTF-8: {0}")]
    Utf8(#[source] std::str::Utf8Error),
    /// Base64 PSBT text is invalid
    #[error("Base64 PSBT is invalid: {0}")]
    Base64(#[source] bitcoin::psbt::PsbtParseError),
    /// Binary PSBT data is invalid
    #[error("binary PSBT is invalid: {0}")]
    Binary(#[source] bitcoin::psbt::Error),
    /// A QR frame is invalid
    #[error("PSBT QR data is invalid: {0}")]
    Qr(#[from] crate::QrTransportError),
    /// Multipart QR input did not contain enough frames
    #[error("PSBT QR data is incomplete")]
    IncompleteQr,
}

#[cfg(test)]
mod tests {
    use bitcoin::{Psbt, Transaction, absolute::LockTime, transaction::Version};

    use super::{PsbtFileFormat, decode_psbt, write_psbt};

    fn empty_psbt() -> Psbt {
        Psbt::from_unsigned_tx(Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        })
        .expect("empty transaction is unsigned")
    }

    #[test]
    fn file_decoder_accepts_binary_base64_and_qr_documents() {
        let psbt = empty_psbt();
        assert_eq!(decode_psbt(&psbt.serialize()).expect("binary PSBT"), psbt);
        assert_eq!(
            decode_psbt(psbt.to_string().as_bytes()).expect("Base64 PSBT"),
            psbt
        );

        for format in [PsbtFileFormat::Bbqr, PsbtFileFormat::Ur] {
            let path = std::env::temp_dir().join(format!(
                "signer-lab-psbt-file-{format:?}-{}",
                uuid::Uuid::new_v4()
            ));
            write_psbt(&path, &psbt, format).expect("write QR document");
            let bytes = std::fs::read(&path).expect("read QR document");
            assert_eq!(decode_psbt(&bytes).expect("decode QR document"), psbt);
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn file_decoder_rejects_empty_input() {
        assert!(matches!(
            decode_psbt(b" \n"),
            Err(super::PsbtFileError::Empty)
        ));
    }
}
