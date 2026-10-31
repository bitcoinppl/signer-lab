use super::{Page, ScreenshotState};
use crate::app::{SignerLabApp, wallet::WalletPane};

/// Keeps screenshot setup with the page that owns it
pub(crate) fn states() -> Vec<ScreenshotState> {
    vec![
        ScreenshotState {
            name: "create",
            page: Page::Wallet,
            has_wallet: false,
            prepare: None,
        },
        ScreenshotState {
            name: "wallet",
            page: Page::Wallet,
            has_wallet: true,
            prepare: None,
        },
        ScreenshotState {
            name: "create-with-cancel",
            page: Page::Wallet,
            has_wallet: true,
            prepare: Some(|_| {
                Ok(Box::new(|app, _, _| {
                    app.wallet.pane = WalletPane::Create;
                }))
            }),
        },
        ScreenshotState {
            name: "wallet-rename",
            page: Page::Wallet,
            has_wallet: true,
            prepare: Some(|_| {
                Ok(Box::new(|app, window, cx| {
                    app.start_rename_signer(window, cx);
                }))
            }),
        },
        ScreenshotState {
            name: "wallet-recovery-words",
            page: Page::Wallet,
            has_wallet: true,
            prepare: Some(|_| {
                Ok(Box::new(|_, window, cx| {
                    cx.defer_in(window, |_, window, cx| {
                        SignerLabApp::show_recovery_words(super::TEST_WORDS.to_owned(), window, cx);
                    });
                }))
            }),
        },
        ScreenshotState {
            name: "wallet-delete-confirm",
            page: Page::Wallet,
            has_wallet: true,
            prepare: Some(|_| {
                Ok(Box::new(|_, window, cx| {
                    cx.defer_in(window, |app, window, cx| {
                        app.confirm_delete_signer(window, cx);
                    });
                }))
            }),
        },
        ScreenshotState {
            name: "wallet-no-data",
            page: Page::Wallet,
            has_wallet: true,
            prepare: Some(|_| {
                Ok(Box::new(|app, _, _| {
                    app.wallet.view = None;
                }))
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::ScreenshotFixtures;
    use crate::app::{SignerLabApp, wallet::RenameState};
    use gpui_kit::{
        AppContext as _, Entity, TestAppContext, VisualTestContext, WindowOptions, px,
        test::TestAppContextExt as _,
    };
    use std::{sync::Arc, time::Duration};

    fn app_window(
        fixtures: &ScreenshotFixtures,
        cx: &mut TestAppContext,
    ) -> (Entity<SignerLabApp>, VisualTestContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = SignerLabApp::new_state(
                        Arc::clone(&fixtures.repository),
                        fixtures.runtime.handle().clone(),
                        fixtures.signers.clone(),
                        fixtures.teleport_sessions(),
                        window,
                        cx,
                    );

                    app.wallet.view = Some(fixtures.wallet.clone());
                    app
                })
            })
            .expect("fixture window")
        });
        (view, VisualTestContext::from_window(handle, cx))
    }

    #[gpui_kit::test]
    fn account_rows_start_below_header(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (_, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let header = cx
            .debug_bounds("wallet-table-header")
            .expect("table header");
        let legacy = cx.debug_bounds("wallet-account-0").expect("legacy row");

        assert_eq!(header.size.height, px(28.0));
        assert_eq!(legacy.size.height, px(44.0));
        assert_eq!(legacy.top(), header.bottom());
    }

    #[gpui_kit::test]
    async fn rename_keyboard_actions_save_or_cancel(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| view.update(cx, |app, cx| app.start_rename_signer(window, cx)));
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            assert!(matches!(view.read(cx).wallet.rename, RenameState::Idle));
            assert_eq!(
                view.read(cx).selected().expect("wallet").name.to_string(),
                "Alice test"
            );
        });

        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.start_rename_signer(window, cx);
                app.wallet
                    .rename_name
                    .update(cx, |input, cx| input.set_value("Renamed test", window, cx));
            });
        });

        cx.simulate_keystrokes("enter");
        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            !view.read(cx).busy()
        })
        .await;

        cx.update(|_, cx| {
            assert!(matches!(view.read(cx).wallet.rename, RenameState::Idle));
            assert_eq!(
                view.read(cx).selected().expect("wallet").name.to_string(),
                "Renamed test"
            );
        });
    }

    #[gpui_kit::test]
    async fn create_import_clears_recovery_words(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let fixtures = ScreenshotFixtures::new().expect("offline fixture");
        let (view, mut cx) = app_window(&fixtures, cx);
        cx.update(|window, cx| {
            view.update(cx, |app, cx| {
                app.seed_qr.signer_name.update(cx, |input, cx| {
                    input.set_value("Imported test", window, cx);
                });

                app.wallet.import_mnemonic.update(cx, |input, cx| {
                    input.set_value(super::super::TEST_WORDS, window, cx);
                });

                app.import_wallet(window, cx);
            });
        });

        let handle = cx.update(|window, _| window.window_handle());
        cx.wait_for(handle, Duration::from_secs(3), |_, cx| {
            !view.read(cx).busy()
        })
        .await;

        cx.update(|_, cx| {
            let app = view.read(cx);
            assert_eq!(app.signers.len(), 3);
            assert!(app.wallet.import_mnemonic.read(cx).value().is_empty());
        });
    }
}
