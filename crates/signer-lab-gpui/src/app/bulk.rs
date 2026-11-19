use super::{
    SignerLabApp, components, design, error_message, input_with_value,
    state::{AddressTypeLabels, ResultScope, WorkflowOutput, address_types},
    text_area,
};
use bitcoin::{Amount, Txid, bip32::Xpub};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    description_list::DescriptionList,
    form::{Form, field},
    h_flex,
    input::{InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use signer_lab_core::{
    AddressType, BulkDestinationSpec, BulkPlan, BulkShape, CurrentBulkOperation,
    ExternalDescriptor, FixedAddress, InputReservation, LabFeeRate, OperationId, OperationKind,
    OutputCount, PlanBulk, PlannedBulkOperation, PlannedTransaction,
};
use std::{str::FromStr as _, sync::Arc};

const BULK_KINDS: [OperationKind; 3] = [
    OperationKind::FanOut,
    OperationKind::Batch,
    OperationKind::Consolidate,
];

const fn kind_label(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::FanOut => "Fan-out",
        OperationKind::Batch => "Batch",
        OperationKind::Consolidate => "Consolidate",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DestinationMode {
    FixedAddress,
    AccountXpub,
    Descriptor,
    SavedSigner,
}

impl DestinationMode {
    const ALL: [Self; 4] = [
        Self::FixedAddress,
        Self::AccountXpub,
        Self::Descriptor,
        Self::SavedSigner,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::FixedAddress => "Address",
            Self::AccountXpub => "XPUB",
            Self::Descriptor => "Descriptor",
            Self::SavedSigner => "Wallet",
        }
    }
}

pub(super) enum Reservation {
    Held,
    Failed(SharedString),
}

pub(super) enum BulkWorkflow {
    Draft,
    Planned {
        operation_id: OperationId,
        plan: BulkPlan,
        reservation: Reservation,
    },
    Failed {
        operation_id: OperationId,
        plan: BulkPlan,
        accepted_txids: Vec<Txid>,
        error: SharedString,
    },
    Broadcast {
        plan: BulkPlan,
    },
}

impl BulkWorkflow {
    pub(super) const fn operation_id(&self) -> Option<OperationId> {
        match self {
            Self::Draft | Self::Broadcast { .. } => None,
            Self::Planned { operation_id, .. } | Self::Failed { operation_id, .. } => {
                Some(*operation_id)
            }
        }
    }

    pub(super) const fn plan(&self) -> Option<&BulkPlan> {
        match self {
            Self::Planned { plan, .. }
            | Self::Failed { plan, .. }
            | Self::Broadcast { plan, .. } => Some(plan),
            Self::Draft => None,
        }
    }
}

impl From<PlannedBulkOperation> for BulkWorkflow {
    fn from(operation: PlannedBulkOperation) -> Self {
        let reservation = match operation.reservation {
            InputReservation::Held => Reservation::Held,
            InputReservation::Failed(error) => Reservation::Failed(error.into()),
        };
        Self::Planned {
            operation_id: operation.operation_id,
            plan: operation.plan,
            reservation,
        }
    }
}

enum ExecutionOutcome {
    Broadcast(signer_lab_core::BulkExecutionReport),
    Error {
        error: String,
        current: Result<Option<CurrentBulkOperation>, String>,
    },
}

pub(super) struct BulkForm {
    pub(super) mode: OperationKind,
    pub(super) destination_mode: DestinationMode,
    pub(super) address_type: AddressType,
    pub(super) destination_signer: Option<signer_lab_core::SignerId>,
    pub(super) destination: Entity<TextareaState>,
    pub(super) count: Entity<InputState>,
    pub(super) amount: Entity<InputState>,
    pub(super) fee_rate: Entity<InputState>,
    pub(super) start_index: Entity<InputState>,
}

pub(super) struct BulkPage {
    pub(super) form: BulkForm,
    pub(super) workflow: BulkWorkflow,
}

impl BulkPage {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SignerLabApp>) -> Self {
        Self {
            form: BulkForm {
                mode: OperationKind::FanOut,
                destination_mode: DestinationMode::FixedAddress,
                address_type: AddressType::NativeSegwit,
                destination_signer: None,
                destination: text_area(window, cx, "Destination"),
                count: input_with_value(window, cx, "Count", "10"),
                amount: input_with_value(window, cx, "Amount", "1000"),
                start_index: input_with_value(window, cx, "Start index", "0"),
                fee_rate: input_with_value(window, cx, "Fee rate", "0.5"),
            },
            workflow: BulkWorkflow::Draft,
        }
    }

    pub(super) fn reset_workflow(&mut self) {
        self.workflow = BulkWorkflow::Draft;
        self.form.destination_signer = None;
    }
}

impl SignerLabApp {
    pub(super) fn bulk_request(&self, cx: &App) -> Result<PlanBulk, String> {
        let signer = self
            .selected()
            .ok_or_else(|| "No wallet selected".to_owned())?;
        let form = &self.bulk.form;

        let shape = if form.mode == OperationKind::Consolidate {
            BulkShape::consolidate()
        } else {
            let count = OutputCount::try_from(
                form.count
                    .read(cx)
                    .value()
                    .parse::<usize>()
                    .map_err(error_message)?,
            )
            .map_err(error_message)?;

            let amount = Amount::from_sat(
                form.amount
                    .read(cx)
                    .value()
                    .parse::<u64>()
                    .map_err(error_message)?,
            );

            match form.mode {
                OperationKind::FanOut => BulkShape::fan_out(count, amount),
                OperationKind::Batch => BulkShape::batch(count, amount),
                OperationKind::Consolidate => unreachable!("consolidation has no output fields"),
            }
            .map_err(error_message)?
        };

        let fee_rate =
            LabFeeRate::from_str(form.fee_rate.read(cx).value().as_ref()).map_err(error_message)?;
        let value = form.destination.read(cx).value();

        let destination = match form.destination_mode {
            DestinationMode::FixedAddress => BulkDestinationSpec::Fixed(
                FixedAddress::parse(value.as_ref(), self.network).map_err(error_message)?,
            ),
            DestinationMode::AccountXpub => BulkDestinationSpec::AccountXpub {
                xpub: Xpub::from_str(value.as_ref()).map_err(error_message)?,
                address_type: form.address_type,
                start_index: form
                    .start_index
                    .read(cx)
                    .value()
                    .parse::<u32>()
                    .map_err(error_message)?,
            },
            DestinationMode::Descriptor => BulkDestinationSpec::Descriptor {
                descriptor: ExternalDescriptor::parse(value.as_ref(), self.network)
                    .map_err(error_message)?,
                start_index: form
                    .start_index
                    .read(cx)
                    .value()
                    .parse::<u32>()
                    .map_err(error_message)?,
            },
            DestinationMode::SavedSigner => BulkDestinationSpec::SavedSigner {
                signer_id: form
                    .destination_signer
                    .ok_or_else(|| "Choose a wallet".to_owned())?,
                address_type: form.address_type,
            },
        };
        Ok(PlanBulk {
            signer,
            network: self.network,
            shape,
            destination,
            fee_rate,
        })
    }

    pub(super) fn accept_current_bulk_operation(
        &mut self,
        operation: Option<CurrentBulkOperation>,
    ) {
        self.bulk.workflow = match operation {
            None => BulkWorkflow::Draft,
            Some(CurrentBulkOperation::Active { operation_id, plan }) => BulkWorkflow::Planned {
                operation_id,
                plan,
                reservation: Reservation::Held,
            },
            Some(CurrentBulkOperation::Failed {
                operation_id,
                plan,
                accepted_txids,
                error,
                ..
            }) => BulkWorkflow::Failed {
                operation_id,
                plan,
                accepted_txids,
                error: error.into(),
            },
        };

        if let Some(plan) = self.bulk.workflow.plan() {
            self.bulk.form.mode = plan.kind();
        }
    }

    fn plan_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !matches!(self.bulk.workflow, BulkWorkflow::Draft) {
            return;
        }

        let request = match self.bulk_request(cx) {
            Ok(request) => request,
            Err(error) => {
                self.set_error(error, window, cx);
                return;
            }
        };

        let repository = Arc::clone(&self.repository);
        self.run(
            "Planning bulk transactions",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                Ok(repository
                    .lock()
                    .await
                    .plan_bulk_operation(request)
                    .await
                    .map_err(error_message))
            },
            |app, result, window, cx| {
                match result {
                    Ok(operation) => {
                        app.bulk.workflow = operation.into();
                    }
                    Err(error) => app.set_error(error, window, cx),
                }

                app.refresh_wallet(window, cx);
            },
        );
    }

    fn execute_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !matches!(self.bulk.workflow, BulkWorkflow::Planned { .. }) {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };

        let Some(operation_id) = self.bulk.workflow.operation_id() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        let reload = Arc::clone(&repository);
        let network = self.network;
        let signer_id = signer.id;
        let reload_signer = signer.clone();
        // signed operations are durable; closing the view must not abort a broadcast in flight
        let execution = self.runtime.spawn(async move {
            repository
                .lock()
                .await
                .execute_bulk_operation(&signer, network, operation_id)
                .await
                .map_err(error_message)
        });
        self.run(
            "Signing and broadcasting",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let result = execution
                    .await
                    .map_err(error_message)
                    .and_then(|result| result);
                let error = match result {
                    Ok(report) => return Ok(ExecutionOutcome::Broadcast(report)),
                    Err(error) => error,
                };

                let mut repository = reload.lock().await;
                let current = match repository
                    .reconcile_bulk_reservations(&reload_signer, network)
                    .await
                {
                    Ok(_) => repository
                        .current_bulk_operation(signer_id, network)
                        .await
                        .map_err(error_message),
                    Err(error) => Err(error_message(error)),
                };
                Ok(ExecutionOutcome::Error { error, current })
            },
            move |app, outcome, window, cx| {
                app.accept_bulk_execution(outcome, window, cx);
            },
        );
    }

    fn accept_bulk_execution(
        &mut self,
        outcome: ExecutionOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            ExecutionOutcome::Broadcast(report) => {
                let summary = report
                    .txids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                let Some(plan) = self.bulk.workflow.plan().cloned() else {
                    return;
                };

                self.bulk.workflow = BulkWorkflow::Broadcast { plan };
                self.set_output(
                    Some(WorkflowOutput::Bulk {
                        summary: summary.into(),
                    }),
                    window,
                    cx,
                );

                self.set_status("Broadcast", cx);
            }
            ExecutionOutcome::Error { mut error, current } => {
                match current {
                    Ok(operation) => self.accept_current_bulk_operation(operation),
                    Err(load_error) => {
                        error.push_str("; operation load failed: ");
                        error.push_str(&load_error);
                    }
                }

                self.set_error(error, window, cx);
            }
        }

        self.refresh_wallet(window, cx);
    }

    fn dismiss_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }

        let Some(signer_id) = self.selected_signer else {
            return;
        };

        let BulkWorkflow::Failed { operation_id, .. } = self.bulk.workflow else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        self.run(
            "Dismiss",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                let mut repository = repository.lock().await;
                repository
                    .acknowledge_failed_bulk_operation(signer_id, operation_id)
                    .await
                    .map_err(error_message)?;
                repository
                    .current_bulk_operation(signer_id, network)
                    .await
                    .map_err(error_message)
            },
            |app, operation, _, cx| {
                app.accept_current_bulk_operation(operation);
                cx.notify();
            },
        );
    }

    fn cancel_bulk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signer) = self.selected() else {
            return;
        };

        let Some(id) = self.bulk.workflow.operation_id() else {
            return;
        };

        let repository = Arc::clone(&self.repository);
        let network = self.network;
        self.run(
            "Cancelling bulk operation",
            ResultScope::Wallet,
            window,
            cx,
            async move {
                Ok(repository
                    .lock()
                    .await
                    .cancel_bulk_operation(&signer, network, id)
                    .await
                    .map_err(error_message))
            },
            |app, result, window, cx| {
                match result {
                    Ok(()) => {
                        app.bulk.reset_workflow();
                        app.set_output(None, window, cx);
                        app.set_status("Bulk operation cancelled and inputs released", cx);
                    }
                    Err(error) => app.set_error(error, window, cx),
                }

                app.refresh_wallet(window, cx);
            },
        );
    }

    pub(super) fn bulk_page(&self, cx: &Context<Self>) -> impl IntoElement {
        if self.selected().is_none() {
            return Self::no_wallet(cx).into_any_element();
        }

        let form = &self.bulk.form;
        let disabled = self.busy() || !matches!(self.bulk.workflow, BulkWorkflow::Draft);
        let transaction = self.bulk_transaction_form(disabled, cx);
        let destination = self.bulk_destination_form(disabled, cx);
        let mut fields = v_flex()
            .p(px(design::CONTENT_PADDING))
            .gap_5()
            .child(components::group_box("Transaction").child(transaction))
            .child(components::group_box("Destination").child(destination));
        if form.mode != OperationKind::Consolidate {
            fields = fields.child(
                components::group_box("Outputs").child(
                    Form::horizontal()
                        .label_width(px(120.0))
                        .child(
                            field()
                                .label("Count")
                                .child(components::text_input(&form.count).disabled(disabled)),
                        )
                        .child(
                            field().label("Amount").child(
                                components::text_input(&form.amount)
                                    .suffix("sats")
                                    .disabled(disabled),
                            ),
                        ),
                ),
            );
        }
        fields = fields.child(
            h_flex().justify_end().child(
                Button::new("bulk-plan")
                    .primary()
                    .label("Plan")
                    .disabled(disabled)
                    .on_click(cx.listener(|app, _, window, cx| app.plan_bulk(window, cx))),
            ),
        );
        h_flex()
            .size_full()
            .child(
                div()
                    .id("bulk-form")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scrollbar()
                    .child(fields),
            )
            .child(self.bulk_summary(cx))
            .into_any_element()
    }

    fn bulk_transaction_form(&self, disabled: bool, cx: &Context<Self>) -> Form {
        let form = &self.bulk.form;
        Form::horizontal()
            .label_width(px(120.0))
            .child(field().label("Type").child(h_flex().items_start().child(
                components::segmented(
                    "bulk-type",
                    BULK_KINDS.into_iter().map(|mode| components::Segment {
                        value: mode,
                        label: (kind_label(mode)).into(),
                        dot: None,
                    }),
                    form.mode,
                    disabled,
                    |app, value, _, cx| {
                        app.bulk.form.mode = value;
                        cx.notify();
                    },
                    cx,
                ),
            )))
            .child(
                field().label("Fee rate").child(
                    components::text_input(&form.fee_rate)
                        .suffix("sat/vB")
                        .disabled(disabled),
                ),
            )
    }

    fn bulk_destination_form(&self, disabled: bool, cx: &Context<Self>) -> Form {
        let form = &self.bulk.form;
        let mut destination = Form::horizontal().label_width(px(120.0)).child(
            field().label("Source").child(
                h_flex().items_start().child(components::segmented(
                    "destination-type",
                    DestinationMode::ALL
                        .into_iter()
                        .map(|mode| components::Segment {
                            value: mode,
                            label: (mode.label()).into(),
                            dot: None,
                        }),
                    form.destination_mode,
                    disabled,
                    |app, value, _, cx| {
                        app.bulk.form.destination_mode = value;
                        cx.notify();
                    },
                    cx,
                )),
            ),
        );

        if matches!(
            form.destination_mode,
            DestinationMode::AccountXpub | DestinationMode::SavedSigner
        ) {
            destination = destination.child(field().label("Address type").child(
                h_flex().items_start().child(components::segmented(
                    "destination-account",
                    address_types().into_iter().map(|kind| components::Segment {
                        value: kind,
                        label: (kind.short_label()).into(),
                        dot: None,
                    }),
                    form.address_type,
                    disabled,
                    |app, value, _, cx| {
                        app.bulk.form.address_type = value;
                        cx.notify();
                    },
                    cx,
                )),
            ));
        }

        if matches!(
            form.destination_mode,
            DestinationMode::AccountXpub | DestinationMode::Descriptor
        ) {
            destination = destination.child(
                field()
                    .label("Start index")
                    .child(components::text_input(&form.start_index).disabled(disabled)),
            );
        }

        if form.destination_mode == DestinationMode::SavedSigner {
            destination = destination.child(self.bulk_destination_wallet(disabled, cx));
        } else {
            destination = destination.child(
                field().label("Destination").child(
                    Textarea::new(&form.destination)
                        .h(px(64.0))
                        .font_family(cx.theme().mono_font_family.clone())
                        .disabled(disabled),
                ),
            );
        }
        destination
    }

    fn bulk_destination_wallet(
        &self,
        disabled: bool,
        cx: &Context<Self>,
    ) -> gpui_kit::component::form::Field {
        let form = &self.bulk.form;
        let wallets = self
            .signers
            .iter()
            .filter(|signer| Some(signer.id) != self.selected_signer)
            .cloned()
            .collect::<Vec<_>>();
        let entity = cx.entity().downgrade();

        let name = form
            .destination_signer
            .and_then(|id| self.signers.iter().find(|signer| signer.id == id))
            .map_or_else(
                || "Choose a wallet".to_owned(),
                |signer| signer.name.to_string(),
            );

        // a menu button keeps the whole field clickable; DropdownButton splits off an unrelated action
        field().label("Destination").child(
            Button::new("destination-wallet")
                .outline()
                .label(name)
                .dropdown_caret(true)
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for signer in &wallets {
                        let id = signer.id;
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(signer.name.to_string()).on_click(
                            move |_, _, cx| {
                                let _ = entity.update(cx, |app, cx| {
                                    if !app.busy()
                                        && matches!(app.bulk.workflow, BulkWorkflow::Draft)
                                    {
                                        app.bulk.form.destination_signer = Some(id);
                                        cx.notify();
                                    }
                                });
                            },
                        ));
                    }
                    menu
                }),
        )
    }

    fn bulk_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut summary = v_flex()
            .id("bulk-summary")
            .w(px(360.0))
            .h_full()
            .flex_shrink_0()
            .p_4()
            .gap_4()
            .border_l_1()
            .border_color(cx.theme().border)
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .justify_between()
                    .child(components::caption("Plan", cx))
                    .child(self.bulk_tag(cx)),
            );
        if let Some(plan) = self.bulk.workflow.plan() {
            summary = summary.child(Self::bulk_plan_details(plan, cx));
        }

        match &self.bulk.workflow {
            BulkWorkflow::Draft => {}
            BulkWorkflow::Planned { reservation, .. } => {
                if let Reservation::Failed(error) = reservation {
                    summary =
                        summary.child(Alert::warning("bulk-reservation-error", error.clone()));
                }
                summary = summary
                    .child(
                        Button::new("bulk-execute")
                            .primary()
                            .w_full()
                            .label("Sign and broadcast")
                            .disabled(self.busy())
                            .on_click(
                                cx.listener(|app, _, window, cx| app.execute_bulk(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("bulk-cancel")
                            .danger()
                            .outline()
                            .w_full()
                            .label("Cancel and release")
                            .disabled(self.busy())
                            .on_click(
                                cx.listener(|app, _, window, cx| app.cancel_bulk(window, cx)),
                            ),
                    );
            }

            BulkWorkflow::Failed {
                error,
                accepted_txids,
                ..
            } => {
                summary = summary.child(Alert::error("bulk-error", error.clone()));
                if !accepted_txids.is_empty() {
                    summary = summary.child(components::caption("Accepted", cx));
                    for (index, txid) in accepted_txids.iter().enumerate() {
                        summary = summary.child(
                            components::mono_value(("bulk-accepted", index), txid.to_string(), cx)
                                .truncated(),
                        );
                    }
                }
                summary = summary.child(
                    Button::new("bulk-dismiss")
                        .outline()
                        .label("Dismiss")
                        .disabled(self.busy())
                        .on_click(cx.listener(|app, _, window, cx| app.dismiss_bulk(window, cx))),
                );
            }
            BulkWorkflow::Broadcast { .. } => {
                summary = summary.child(self.bulk_broadcast_result(cx));
            }
        }
        summary
    }

    fn bulk_tag(&self, cx: &Context<Self>) -> Tag {
        // outlined tags keep state text readable when semantic foreground tokens match the fill
        match &self.bulk.workflow {
            BulkWorkflow::Planned {
                reservation: Reservation::Held,
                ..
            } => Tag::warning().outline().child(
                div()
                    .text_color(cx.theme().foreground)
                    .child("Inputs reserved"),
            ),
            BulkWorkflow::Draft => Tag::secondary().child("Draft"),
            BulkWorkflow::Planned {
                reservation: Reservation::Failed(_),
                ..
            } => Tag::danger().outline().child(
                div()
                    .text_color(cx.theme().foreground)
                    .child("Not reserved"),
            ),
            BulkWorkflow::Failed { .. } => Tag::danger()
                .outline()
                .child(div().text_color(cx.theme().foreground).child("Failed")),
            BulkWorkflow::Broadcast { .. } => Tag::success()
                .outline()
                .child(div().text_color(cx.theme().foreground).child("Broadcast")),
        }
    }

    fn bulk_plan_details(plan: &BulkPlan, cx: &Context<Self>) -> impl IntoElement {
        let mut summary = v_flex().gap_4();
        let inputs: usize = plan
            .transactions()
            .iter()
            .map(PlannedTransaction::input_count)
            .sum();
        let outputs: usize = plan
            .transactions()
            .iter()
            .map(PlannedTransaction::output_count)
            .sum();
        let rate = plan.transactions().first().map_or_else(
            || "0".to_owned(),
            |transaction| {
                let kwu = transaction.estimated_fee_rate().to_sat_per_kwu();
                let fraction = format!("{:03}", (kwu % 250) * 4);
                let fraction = fraction.trim_end_matches('0');
                if fraction.is_empty() {
                    return (kwu / 250).to_string();
                }
                format!("{}.{fraction}", kwu / 250)
            },
        );

        let mut details = DescriptionList::horizontal()
            .label_width(px(110.0))
            .columns(1)
            .bordered(false)
            .item("Transactions", plan.transactions().len().to_string(), 1)
            .item("Inputs", inputs.to_string(), 1)
            .item("Outputs", outputs.to_string(), 1)
            .item("Fee rate", format!("{rate} sat/vB"), 1);
        if let Some(range) = plan.destination_range() {
            details = details.item("Addresses", format!("{}–{}", range.start(), range.end()), 1);
        }

        summary = summary.child(details);
        for (index, transaction) in plan.transactions().iter().enumerate() {
            summary = summary.child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("#{}", index + 1)),
                    )
                    .child(format!(
                        "{} → {}",
                        transaction.input_count(),
                        transaction.output_count()
                    ))
                    .child(components::amount_sats(transaction.fee().to_sat())),
            );
        }
        summary
    }

    fn bulk_broadcast_result(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut summary = v_flex().gap_4();
        if matches!(self.visible_output(), Some(WorkflowOutput::Bulk { .. })) {
            summary = summary.child(components::payload_panel(
                "Result",
                &self.display_output,
                Button::new("bulk-copy")
                    .outline()
                    .small()
                    .label("Copy")
                    .on_click(cx.listener(|app, _, _, cx| app.copy_output(cx))),
                cx,
            ));
        }
        summary.child(
            Button::new("bulk-new")
                .outline()
                .label("New plan")
                .disabled(self.busy())
                .on_click(cx.listener(|app, _, window, cx| {
                    app.bulk.reset_workflow();
                    app.set_output(None, window, cx);
                })),
        )
    }
}
