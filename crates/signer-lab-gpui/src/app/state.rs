use gpui_kit::{Image, SharedString, Task};
use signer_lab_core::{
    AddressType, ExportArtifact, ExportPayload, LabNetwork, QrFrames, SignedPsbt, SignerId,
    SyncControl,
};
use std::{cell::RefCell, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Wallet,
    SeedQr,
    Export,
    Sign,
    Bulk,
}

impl Page {
    pub(super) const ALL: [Self; 5] = [
        Self::Wallet,
        Self::SeedQr,
        Self::Export,
        Self::Sign,
        Self::Bulk,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Wallet => "Wallet",
            Self::SeedQr => "Seed QR",
            Self::Export => "Export",
            Self::Sign => "Sign",
            Self::Bulk => "Bulk",
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
    Signed {
        signed: SignedPsbt,
        playback: QrPlayback,
    },
    Bulk {
        summary: SharedString,
    },
}

impl WorkflowOutput {
    pub(super) const fn page(&self) -> Page {
        match self {
            Self::Export { .. } => Page::Export,
            Self::SeedQr { .. } => Page::SeedQr,
            Self::Signed { .. } => Page::Sign,
            Self::Bulk { .. } => Page::Bulk,
        }
    }

    pub(super) fn playback_mut(&mut self) -> Option<&mut QrPlayback> {
        match self {
            Self::Export { playback, .. } => playback.as_mut(),
            Self::SeedQr { playback } | Self::Signed { playback, .. } => Some(playback),
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
            Self::Signed { signed, .. } => signed.psbt().to_string(),
            Self::Bulk { summary } => summary.to_string(),
        })
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
        frames: Vec<String>,
        index: usize,
        interval_ms: u64,
        animation: QrAnimation,
    },
}

impl QrSequence {
    pub(super) fn static_frame(frame: String) -> Self {
        Self::Static(frame)
    }

    pub(super) fn animated(qr: QrFrames) -> Option<Self> {
        if qr.frames.is_empty() {
            return None;
        }
        Some(Self::Animated {
            frames: qr.frames,
            index: 0,
            interval_ms: qr.interval_ms,
            animation: QrAnimation::Playing,
        })
    }

    pub(super) fn current_frame(&self) -> &str {
        match self {
            Self::Static(frame) => frame,
            Self::Animated { frames, index, .. } => &frames[*index],
        }
    }

    pub(super) const fn frame_index(&self) -> usize {
        match self {
            Self::Static(_) => 0,
            Self::Animated { index, .. } => *index,
        }
    }

    pub(super) fn frame_count(&self) -> usize {
        match self {
            Self::Static(_) => 1,
            Self::Animated { frames, .. } => frames.len(),
        }
    }

    pub(super) fn animation_interval_ms(&self) -> Option<u64> {
        match self {
            Self::Animated {
                frames,
                interval_ms,
                animation: QrAnimation::Playing,
                ..
            } if frames.len() > 1 => Some(*interval_ms),
            Self::Static(_) | Self::Animated { .. } => None,
        }
    }

    pub(super) const fn animation(&self) -> Option<QrAnimation> {
        match self {
            Self::Animated {
                frames, animation, ..
            } if frames.len() > 1 => Some(*animation),
            Self::Static(_) | Self::Animated { .. } => None,
        }
    }

    pub(super) fn toggle_animation(&mut self) -> Option<QrAnimation> {
        let Self::Animated {
            frames, animation, ..
        } = self
        else {
            return None;
        };

        if frames.len() < 2 {
            return None;
        }

        *animation = animation.toggled();
        Some(*animation)
    }

    pub(super) fn advance(&mut self) {
        let Self::Animated { frames, index, .. } = self else {
            return;
        };

        *index = (*index + 1) % frames.len();
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

    pub(super) fn frame_count(&self) -> usize {
        self.sequence.frame_count()
    }

    pub(super) fn animation_interval_ms(&self) -> Option<u64> {
        self.sequence.animation_interval_ms()
    }

    pub(super) const fn animation(&self) -> Option<QrAnimation> {
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

        let image = crate::qr_image::render(self.current_frame(), u32::from(side))?;
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
    use signer_lab_core::{QrFrames, QrTransport};
    use std::sync::Arc;

    #[test]
    fn playback_pauses_and_invalidates_only_changed_images() {
        let mut playback = QrPlayback::animated(QrFrames {
            transport: QrTransport::Bbqr,
            frames: vec!["first".into(), "second".into()],
            interval_ms: 250,
        })
        .expect("frames");

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
}
