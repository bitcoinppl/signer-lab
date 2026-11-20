use bitcoin::consensus::encode::serialize_hex;
use gpui_kit::{Image, SharedString, Task};
use signer_lab_core::{
    AddressType, ExportArtifact, ExportPayload, LabNetwork, QrDensity, QrErrorCorrection,
    QrFramePosition, QrFrames, QrTransport, SignedPsbt, SignerId, SyncControl,
};
use std::{cell::RefCell, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Wallet,
    SeedQr,
    Export,
    Sign,
    Bulk,
    KeyTeleport,
}

impl Page {
    pub(super) const ALL: [Self; 6] = [
        Self::Wallet,
        Self::SeedQr,
        Self::Export,
        Self::Sign,
        Self::Bulk,
        Self::KeyTeleport,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Wallet => "Wallet",
            Self::SeedQr => "Seed QR",
            Self::Export => "Export",
            Self::Sign => "Sign",
            Self::Bulk => "Bulk",
            Self::KeyTeleport => "Key Teleport",
        }
    }
}

pub(super) enum Activity {
    Idle,
    Working(SharedString),
    Syncing { control: SyncControl },
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Self::Syncing { control } = self {
            control.cancel();
        }
    }
}

pub(super) enum Outcome {
    Info(SharedString),
    Error(SharedString),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct WalletScope {
    pub(super) signer: Option<SignerId>,
    pub(super) network: LabNetwork,
}

#[derive(Clone, Copy)]
pub(super) enum ResultScope {
    Wallet,
    Page(Page),
}

/// Output carries its owning page so another workflow cannot display it
pub(super) enum WorkflowOutput {
    Export {
        artifact: ExportArtifact,
        playback: Option<QrPlayback>,
    },
    SeedQr {
        playback: QrPlayback,
    },
    Signed(Box<SignedOutput>),
    Bulk {
        summary: SharedString,
    },
}

impl WorkflowOutput {
    pub(super) const fn page(&self) -> Page {
        match self {
            Self::Export { .. } => Page::Export,
            Self::SeedQr { .. } => Page::SeedQr,
            Self::Signed(_) => Page::Sign,
            Self::Bulk { .. } => Page::Bulk,
        }
    }

    pub(super) fn playback_mut(&mut self) -> Option<&mut QrPlayback> {
        match self {
            Self::Export { playback, .. } => playback.as_mut(),
            Self::SeedQr { playback } => Some(playback),
            Self::Signed(output) => Some(&mut output.playback),
            Self::Bulk { .. } => None,
        }
    }

    pub(super) fn text(&self) -> Option<String> {
        Some(match self {
            Self::Export { artifact, .. } => match &artifact.payload {
                ExportPayload::Text(text) => text.clone(),
                ExportPayload::Ur { frames, .. } => frames.join("\n"),
            },
            Self::SeedQr { .. } => return None,
            Self::Signed(output) => output.text(),
            Self::Bulk { summary } => summary.to_string(),
        })
    }
}

/// What the signed QR and text carry
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SignedFormat {
    /// The finalized transaction, which a watch-only wallet can broadcast as is
    #[default]
    Transaction,
    /// The signed PSBT, for flows that still combine or finalize it
    Psbt(QrTransport),
}

impl SignedFormat {
    pub(super) const ALL: [Self; 3] = [
        Self::Transaction,
        Self::Psbt(QrTransport::Bbqr),
        Self::Psbt(QrTransport::Ur),
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Transaction => "Transaction",
            Self::Psbt(QrTransport::Bbqr) => "PSBT · BBQR",
            Self::Psbt(QrTransport::Ur) => "PSBT · UR",
        }
    }

    /// Whether `signed` can be shown in this format
    pub(super) const fn available(self, signed: &SignedPsbt) -> bool {
        !matches!(self, Self::Transaction) || signed.transaction().is_some()
    }
}

/// Signed QR frames built off the UI thread, before the main thread adds playback
///
/// Only [`EncodedSigned::new`] picks the format, so a transaction format always has a transaction
pub(super) struct EncodedSigned {
    signed: SignedPsbt,
    format: SignedFormat,
    frames: QrFrames,
}

impl EncodedSigned {
    /// Encodes `preferred`, or a BBQR PSBT while some inputs still lack final scripts
    pub(super) fn new(signed: SignedPsbt, preferred: SignedFormat) -> Result<Self, String> {
        let density = QrDensity::default();
        let (format, frames) = match (preferred, signed.transaction()) {
            (SignedFormat::Transaction, Some(transaction)) => (
                SignedFormat::Transaction,
                QrFrames::from_transaction(transaction, density),
            ),
            (SignedFormat::Transaction, None) => (
                SignedFormat::Psbt(QrTransport::Bbqr),
                signed.qr_frames(QrTransport::Bbqr, density),
            ),
            (SignedFormat::Psbt(transport), _) => (
                SignedFormat::Psbt(transport),
                signed.qr_frames(transport, density),
            ),
        };

        Ok(Self {
            signed,
            format,
            frames: frames.map_err(|error| error.to_string())?,
        })
    }
}

/// A signed PSBT with the format and QR frames on screen
pub(super) struct SignedOutput {
    signed: SignedPsbt,
    format: SignedFormat,
    playback: QrPlayback,
}

impl SignedOutput {
    /// Returns `None` when the encoder produced no frames
    pub(super) fn new(encoded: EncodedSigned) -> Option<Self> {
        Some(Self {
            playback: QrPlayback::animated(encoded.frames)?,
            signed: encoded.signed,
            format: encoded.format,
        })
    }

    pub(super) const fn signed(&self) -> &SignedPsbt {
        &self.signed
    }

    pub(super) const fn format(&self) -> SignedFormat {
        self.format
    }

    pub(super) const fn playback(&self) -> &QrPlayback {
        &self.playback
    }

    fn text(&self) -> String {
        match (self.format, self.signed.transaction()) {
            (SignedFormat::Transaction, Some(transaction)) => serialize_hex(transaction),
            _ => self.signed.psbt().to_string(),
        }
    }
}

pub(super) const fn address_types() -> [AddressType; 3] {
    AddressType::ALL
}

/// UI labels stay separate from the core address-type model
pub(super) trait AddressTypeLabels {
    /// Names an address type in compact controls
    fn short_label(self) -> &'static str;
    /// Names an address type in account details
    fn full_label(self) -> &'static str;
    /// Names the account script format
    fn script_label(self) -> &'static str;
}

impl AddressTypeLabels for AddressType {
    fn short_label(self) -> &'static str {
        match self {
            Self::Legacy => "Legacy",
            Self::WrappedSegwit => "Nested",
            Self::NativeSegwit => "Native",
        }
    }

    fn full_label(self) -> &'static str {
        match self {
            Self::Legacy => "Legacy",
            Self::WrappedSegwit => "Nested SegWit",
            Self::NativeSegwit => "Native SegWit",
        }
    }

    fn script_label(self) -> &'static str {
        match self {
            Self::Legacy => "P2PKH",
            Self::WrappedSegwit => "P2SH-P2WPKH",
            Self::NativeSegwit => "P2WPKH",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum QrPreviewSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl QrPreviewSize {
    pub(super) const fn side(self) -> u16 {
        match self {
            Self::Small => 180,
            Self::Medium => 260,
            Self::Large => 336,
        }
    }

    pub(super) const fn smaller(self) -> Self {
        match self {
            Self::Small | Self::Medium => Self::Small,
            Self::Large => Self::Medium,
        }
    }

    pub(super) const fn larger(self) -> Self {
        match self {
            Self::Small => Self::Medium,
            Self::Medium | Self::Large => Self::Large,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QrAnimation {
    Playing,
    Paused,
}

impl QrAnimation {
    const fn toggled(self) -> Self {
        match self {
            Self::Playing => Self::Paused,
            Self::Paused => Self::Playing,
        }
    }
}

enum QrSequence {
    Static(String),
    Animated {
        frames: QrFrames,
        index: usize,
        animation: QrAnimation,
    },
}

impl QrSequence {
    pub(super) fn static_frame(frame: String) -> Self {
        Self::Static(frame)
    }

    pub(super) fn animated(frames: QrFrames) -> Option<Self> {
        if frames.frames().is_empty() {
            return None;
        }

        Some(Self::Animated {
            frames,
            index: 0,
            animation: QrAnimation::Playing,
        })
    }

    pub(super) fn current_frame(&self) -> &str {
        match self {
            Self::Static(frame) => frame,
            Self::Animated { frames, index, .. } => &frames.frames()[*index],
        }
    }

    pub(super) const fn frame_index(&self) -> usize {
        match self {
            Self::Static(_) => 0,
            Self::Animated { index, .. } => *index,
        }
    }

    /// Static frames are `SeedQR` digits and BBQR text, both sized for level L
    pub(super) const fn error_correction(&self) -> QrErrorCorrection {
        match self {
            Self::Static(_) => QrErrorCorrection::Low,
            Self::Animated { frames, .. } => frames.error_correction(),
        }
    }

    pub(super) fn position(&self) -> Option<QrFramePosition> {
        match self {
            Self::Static(_) => None,
            Self::Animated { frames, index, .. } => frames.position(*index),
        }
    }

    fn is_multi_frame(&self) -> bool {
        match self {
            Self::Static(_) => false,
            Self::Animated { frames, .. } => frames.frames().len() > 1,
        }
    }

    pub(super) fn animation_interval_ms(&self) -> Option<u64> {
        match self {
            Self::Animated {
                frames,
                animation: QrAnimation::Playing,
                ..
            } if self.is_multi_frame() => Some(frames.interval_ms()),
            Self::Static(_) | Self::Animated { .. } => None,
        }
    }

    pub(super) fn animation(&self) -> Option<QrAnimation> {
        match self {
            Self::Animated { animation, .. } if self.is_multi_frame() => Some(*animation),
            Self::Static(_) | Self::Animated { .. } => None,
        }
    }

    pub(super) fn toggle_animation(&mut self) -> Option<QrAnimation> {
        if !self.is_multi_frame() {
            return None;
        }

        let Self::Animated { animation, .. } = self else {
            return None;
        };

        *animation = animation.toggled();
        Some(*animation)
    }

    pub(super) fn advance(&mut self) {
        let Self::Animated { frames, index, .. } = self else {
            return;
        };

        *index = (*index + 1) % frames.frames().len();
    }
}

struct QrImageCache {
    index: usize,
    side: u16,
    image: Arc<Image>,
}

/// Playback owns both its animation task and its rendered current-frame cache
pub(super) struct QrPlayback {
    sequence: QrSequence,
    cache: RefCell<Option<QrImageCache>>,
    pub(super) timer: Option<Task<()>>,
}

impl QrPlayback {
    pub(super) fn static_frame(frame: String) -> Self {
        Self::new(QrSequence::static_frame(frame))
    }

    pub(super) fn animated(frames: QrFrames) -> Option<Self> {
        QrSequence::animated(frames).map(Self::new)
    }

    fn new(sequence: QrSequence) -> Self {
        Self {
            sequence,
            cache: RefCell::new(None),
            timer: None,
        }
    }

    pub(super) fn current_frame(&self) -> &str {
        self.sequence.current_frame()
    }

    pub(super) const fn frame_index(&self) -> usize {
        self.sequence.frame_index()
    }

    /// Returns `None` for a static frame
    pub(super) fn position(&self) -> Option<QrFramePosition> {
        self.sequence.position()
    }

    pub(super) fn animation_interval_ms(&self) -> Option<u64> {
        self.sequence.animation_interval_ms()
    }

    pub(super) fn animation(&self) -> Option<QrAnimation> {
        self.sequence.animation()
    }

    pub(super) fn toggle_animation(&mut self) -> Option<QrAnimation> {
        self.sequence.toggle_animation()
    }

    pub(super) fn advance(&mut self) {
        self.sequence.advance();
    }

    pub(super) fn image(&self, side: u16) -> Result<Arc<Image>, String> {
        let mut cache = self.cache.borrow_mut();
        if let Some(cached) = cache
            .as_ref()
            .filter(|cached| cached.index == self.frame_index() && cached.side == side)
        {
            return Ok(Arc::clone(&cached.image));
        }

        let image = crate::qr_image::render(
            self.current_frame(),
            self.sequence.error_correction(),
            u32::from(side),
        )?;
        *cache = Some(QrImageCache {
            index: self.frame_index(),
            side,
            image: Arc::clone(&image),
        });
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::{QrAnimation, QrPlayback};
    use crate::qr_image::tests::multipart_ur_frames;
    use signer_lab_core::QrFramePosition;
    use std::sync::Arc;

    #[test]
    fn playback_pauses_and_invalidates_only_changed_images() {
        let mut playback = QrPlayback::animated(multipart_ur_frames()).expect("frames");

        let image = playback.image(260).expect("QR");
        assert!(Arc::ptr_eq(
            &image,
            &playback.image(260).expect("cached QR")
        ));

        assert_eq!(playback.toggle_animation(), Some(QrAnimation::Paused));
        assert_eq!(playback.animation_interval_ms(), None);
        assert_eq!(playback.toggle_animation(), Some(QrAnimation::Playing));
        playback.advance();
        assert!(!Arc::ptr_eq(&image, &playback.image(260).expect("next QR")));
        assert!(!Arc::ptr_eq(
            &playback.image(260).expect("QR"),
            &playback.image(180).expect("small QR")
        ));
    }

    #[test]
    fn playback_counts_fragments_then_fountain_parts_and_loops() {
        let frames = multipart_ur_frames();
        let cycle = frames.frames().len();
        let mut playback = QrPlayback::animated(frames).expect("frames");
        let Some(QrFramePosition::Fragment {
            count: fragments, ..
        }) = playback.position()
        else {
            panic!("a UR cycle starts with its fragment pass");
        };

        for _ in 0..fragments {
            playback.advance();
        }

        assert_eq!(
            playback.position(),
            Some(QrFramePosition::Fountain {
                number: 1,
                count: cycle - fragments,
            })
        );

        for _ in fragments..cycle {
            playback.advance();
        }

        assert_eq!(
            playback.position(),
            Some(QrFramePosition::Fragment {
                number: 1,
                count: fragments,
            })
        );
    }
}
