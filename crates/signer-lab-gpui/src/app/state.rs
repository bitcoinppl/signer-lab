use std::sync::Arc;

use gpui::{Context, Empty, EntityId, Image, IntoElement, Pixels, Render, Window, px};
use signer_lab_core::{
    AccountPublicData, AddressType, BulkPlan, Destination, NetworkWallet, OperationId,
    OperationKind, QrFrames, QrTransport, SignerId, VirtualSigner, WalletAccountSnapshot,
};
use tokio_util::sync::CancellationToken;

use super::error_message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum ResizableTextArea {
    PsbtInput,
    WorkflowOutput,
    SeedQrOutput,
}

impl ResizableTextArea {
    fn minimum_height(self) -> Pixels {
        match self {
            Self::PsbtInput => px(120.0),
            Self::WorkflowOutput => px(96.0),
            Self::SeedQrOutput => px(72.0),
        }
    }

    fn maximum_height(self) -> Pixels {
        match self {
            Self::PsbtInput => px(520.0),
            Self::WorkflowOutput => px(480.0),
            Self::SeedQrOutput => px(240.0),
        }
    }
}

pub(super) struct TextAreaHeights {
    psbt_input: Pixels,
    workflow_output: Pixels,
    seed_qr_output: Pixels,
}

impl Default for TextAreaHeights {
    fn default() -> Self {
        Self {
            psbt_input: px(280.0),
            workflow_output: px(150.0),
            seed_qr_output: px(72.0),
        }
    }
}

impl TextAreaHeights {
    pub(super) fn height(&self, area: ResizableTextArea) -> Pixels {
        match area {
            ResizableTextArea::PsbtInput => self.psbt_input,
            ResizableTextArea::WorkflowOutput => self.workflow_output,
            ResizableTextArea::SeedQrOutput => self.seed_qr_output,
        }
    }

    pub(super) fn resize(&mut self, area: ResizableTextArea, height: Pixels) {
        let height = height.max(area.minimum_height()).min(area.maximum_height());

        match area {
            ResizableTextArea::PsbtInput => self.psbt_input = height,
            ResizableTextArea::WorkflowOutput => self.workflow_output = height,
            ResizableTextArea::SeedQrOutput => self.seed_qr_output = height,
        }
    }
}

#[derive(Clone)]
pub(super) struct TextAreaDrag {
    owner: EntityId,
    area: ResizableTextArea,
    start_y: Pixels,
    start_height: Pixels,
}

impl TextAreaDrag {
    pub(super) fn new(owner: EntityId, area: ResizableTextArea, start_height: Pixels) -> Self {
        Self {
            owner,
            area,
            start_y: px(0.0),
            start_height,
        }
    }

    pub(super) fn started_at(&self, start_y: Pixels) -> Self {
        Self {
            start_y,
            ..self.clone()
        }
    }

    pub(super) fn belongs_to(&self, owner: EntityId, area: ResizableTextArea) -> bool {
        self.owner == owner && self.area == area
    }

    pub(super) fn height_at(&self, current_y: Pixels) -> Pixels {
        self.start_height + current_y - self.start_y
    }
}

impl Render for TextAreaDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Wallet,
    Export,
    Sign,
    Bulk,
    SeedQr,
}

impl Page {
    pub(super) const ALL: [Self; 5] = [
        Self::Wallet,
        Self::Export,
        Self::Sign,
        Self::Bulk,
        Self::SeedQr,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Wallet => "Wallet",
            Self::Export => "Exports",
            Self::Sign => "Sign PSBT",
            Self::Bulk => "Bulk",
            Self::SeedQr => "SeedQR",
        }
    }
}

#[derive(Clone)]
pub(super) struct WalletView {
    pub(super) accounts: Vec<AccountPublicData>,
    pub(super) snapshots: Vec<WalletAccountSnapshot>,
}

impl WalletView {
    pub(super) fn from_wallet(wallet: &NetworkWallet) -> Result<Self, String> {
        let accounts = wallet.public_accounts().map_err(error_message)?;
        let snapshots = address_types()
            .into_iter()
            .map(|address_type| wallet.account(address_type).snapshot())
            .collect();

        Ok(Self {
            accounts,
            snapshots,
        })
    }
}

pub(super) struct CameraState {
    generation: u64,
    control: CancellationToken,
    preview: Option<Arc<Image>>,
}

impl CameraState {
    pub(super) fn new(generation: u64) -> Self {
        Self {
            generation,
            control: CancellationToken::new(),
            preview: None,
        }
    }

    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn control(&self) -> CancellationToken {
        self.control.clone()
    }

    pub(super) fn preview(&self) -> Option<&Arc<Image>> {
        self.preview.as_ref()
    }

    pub(super) fn set_preview(&mut self, preview: Arc<Image>) {
        self.preview = Some(preview);
    }

    pub(super) fn cancel(self) {
        self.control.cancel();
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

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
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

pub(super) enum QrPlayback {
    Static(String),
    Animated {
        transport: QrTransport,
        frames: Vec<String>,
        index: usize,
        interval_ms: u64,
        animation: QrAnimation,
    },
}

impl QrPlayback {
    pub(super) fn static_frame(frame: String) -> Self {
        Self::Static(frame)
    }

    pub(super) fn animated(qr: QrFrames) -> Option<Self> {
        if qr.frames.is_empty() {
            return None;
        }

        Some(Self::Animated {
            transport: qr.transport,
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

    pub(super) fn first_frame(&self) -> &str {
        match self {
            Self::Static(frame) => frame,
            Self::Animated { frames, .. } => &frames[0],
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

    pub(super) const fn transport(&self) -> Option<QrTransport> {
        match self {
            Self::Static(_) => None,
            Self::Animated { transport, .. } => Some(*transport),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum RenameState {
    #[default]
    Idle,
    Editing(SignerId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BulkMode {
    FanOut,
    Batch,
    Consolidate,
}

impl BulkMode {
    pub(super) const ALL: [Self; 3] = [Self::FanOut, Self::Batch, Self::Consolidate];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::FanOut => "Fan out",
            Self::Batch => "Batch",
            Self::Consolidate => "Consolidate",
        }
    }

    pub(super) const fn from_operation_kind(kind: OperationKind) -> Self {
        match kind {
            OperationKind::FanOut => Self::FanOut,
            OperationKind::Batch => Self::Batch,
            OperationKind::Consolidate => Self::Consolidate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DestinationMode {
    FixedAddress,
    AccountXpub,
    Descriptor,
    SavedSigner,
}

impl DestinationMode {
    pub(super) const ALL: [Self; 4] = [
        Self::FixedAddress,
        Self::AccountXpub,
        Self::Descriptor,
        Self::SavedSigner,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::FixedAddress => "One address",
            Self::AccountXpub => "Account XPUB",
            Self::Descriptor => "Descriptor",
            Self::SavedSigner => "Saved signer",
        }
    }
}

#[derive(Clone)]
pub(super) enum BulkDestinationInput {
    Public(Destination),
    SavedSigner(VirtualSigner),
}

/// The mutually exclusive local states of the bulk-operation workflow
///
/// An operation identifier is only available after a plan exists. A failed operation may lack a
/// local plan when recovery loading failed, but it remains cancellable through its persisted ID
pub(super) enum BulkWorkflow {
    Empty,
    Prepared {
        operation_id: OperationId,
        plan: BulkPlan,
    },
    Failed {
        operation_id: OperationId,
        plan: Option<BulkPlan>,
    },
}

impl BulkWorkflow {
    pub(super) const fn empty() -> Self {
        Self::Empty
    }

    pub(super) const fn prepared(operation_id: OperationId, plan: BulkPlan) -> Self {
        Self::Prepared { operation_id, plan }
    }

    pub(super) const fn failed(operation_id: OperationId, plan: Option<BulkPlan>) -> Self {
        Self::Failed { operation_id, plan }
    }

    pub(super) const fn operation_id(&self) -> Option<OperationId> {
        match self {
            Self::Empty => None,
            Self::Prepared { operation_id, .. } | Self::Failed { operation_id, .. } => {
                Some(*operation_id)
            }
        }
    }

    pub(super) const fn plan(&self) -> Option<&BulkPlan> {
        match self {
            Self::Empty => None,
            Self::Prepared { plan, .. } => Some(plan),
            Self::Failed { plan, .. } => plan.as_ref(),
        }
    }

    pub(super) const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

const fn address_types() -> [AddressType; 3] {
    [
        AddressType::Legacy,
        AddressType::WrappedSegwit,
        AddressType::NativeSegwit,
    ]
}

#[cfg(test)]
mod tests {
    use gpui::px;
    use signer_lab_core::{QrFrames, QrTransport};

    use super::{QrAnimation, QrPlayback, ResizableTextArea, TextAreaHeights};

    #[test]
    fn text_area_heights_are_independent_and_stay_usable() {
        let mut heights = TextAreaHeights::default();

        heights.resize(ResizableTextArea::WorkflowOutput, px(20.0));
        assert_eq!(heights.height(ResizableTextArea::WorkflowOutput), px(96.0));
        assert_eq!(heights.height(ResizableTextArea::PsbtInput), px(280.0));

        heights.resize(ResizableTextArea::WorkflowOutput, px(900.0));
        assert_eq!(heights.height(ResizableTextArea::WorkflowOutput), px(480.0));
        assert_eq!(heights.height(ResizableTextArea::SeedQrOutput), px(72.0));
    }

    #[test]
    fn animated_qr_playback_has_explicit_pause_and_resume_transitions() {
        let frames = QrFrames {
            transport: QrTransport::Bbqr,
            frames: vec!["first".to_owned(), "second".to_owned()],
            interval_ms: 250,
        };
        let mut playback = QrPlayback::animated(frames).expect("frames are not empty");

        assert_eq!(playback.animation(), Some(QrAnimation::Playing));
        assert_eq!(playback.animation_interval_ms(), Some(250));

        assert_eq!(playback.toggle_animation(), Some(QrAnimation::Paused));
        assert_eq!(playback.animation_interval_ms(), None);

        assert_eq!(playback.toggle_animation(), Some(QrAnimation::Playing));
        assert_eq!(playback.animation_interval_ms(), Some(250));
    }
}
