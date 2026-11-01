use crate::app::SignerLabApp;
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{ClipboardItem, Context, ParentElement as _, Styled as _, Window, div, px};
use std::sync::Arc;

impl SignerLabApp {
    pub(in crate::app) fn show_recovery_words(
        phrase: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let phrase = Arc::new(phrase);
        window.open_dialog(cx, move |dialog, _, cx| {
            let words: Vec<_> = phrase
                .split_whitespace()
                .enumerate()
                .map(|(index, word)| (index + 1, word.to_owned()))
                .collect();
            let rows = words.chunks(3).map(|row| {
                h_flex().gap_4().children(row.iter().map(|(index, word)| {
                    h_flex()
                        .flex_1()
                        .gap_1()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(13.0))
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{index}.")),
                        )
                        .child(word.clone())
                }))
            });

            let copy = Arc::clone(&phrase);
            dialog
                .title("Recovery words")
                .w(px(520.0))
                .child(v_flex().gap_3().children(rows))
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(Button::new("copy-words").outline().label("Copy").on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()));
                            },
                        ))
                        .child(
                            Button::new("done-words")
                                .primary()
                                .label("Done")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }

    pub(in crate::app) fn confirm_delete_signer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.bulk.workflow.operation_id().is_some() {
            return;
        }

        let Some(signer) = self.selected() else {
            return;
        };

        let entity = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let confirm = entity.clone();
            let delete = entity.clone();
            dialog
                .title(format!("Delete “{}”?", signer.name))
                .child("This removes the wallet and its saved recovery words. Unsaved test coins become unreachable.")
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |app, cx| app.delete_signer(window, cx));
                    true
                })
                .footer(h_flex().gap_2().justify_end()
                    .child(Button::new("delete-cancel").outline().label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("delete-confirm").danger().label("Delete")
                        .on_click(move |_, window, cx| {
                            let _ = delete.update(cx, |app, cx| app.delete_signer(window, cx));
                            window.close_dialog(cx);
                        })))
        });
    }
}
