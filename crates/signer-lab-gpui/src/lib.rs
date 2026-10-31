//! Signer Lab desktop application

mod app;
mod camera;
mod qr_image;
mod secrets;

#[cfg(feature = "test-support")]
pub use app::test_support;

use std::sync::{Arc, Mutex};

use color_eyre::eyre::{Result, eyre};
use directories::ProjectDirs;
use gpui_kit::component::{Theme, TitleBar};
use gpui_kit::{
    AppContext as _, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size,
};
use signer_lab_core::{KeyTeleportSessionStore, LabStore, SignerRepository};

use crate::app::{AppRepository, SignerLabApp, design};

/// Opens the Signer Lab desktop window with its persistent repository
///
/// # Errors
/// Returns an error when startup storage or the native window cannot be opened
pub fn run() -> Result<()> {
    color_eyre::install()?;

    let project_dirs = ProjectDirs::from("org", "bitcoinppl", "Signer Lab")
        .ok_or_else(|| eyre!("application data directory is not available"))?;
    let data_dir = project_dirs.data_local_dir();

    std::fs::create_dir_all(data_dir)?;

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );

    let store = runtime.block_on(LabStore::open_or_create(data_dir.join("metadata.db")))?;
    let repository = SignerRepository::new(
        store,
        secrets::AppSecretStore::Keychain,
        data_dir.join("wallets"),
    );

    let repository: AppRepository = Arc::new(tokio::sync::Mutex::new(repository));
    let signers = runtime.block_on(async { repository.lock().await.signers().await })?;
    let teleport_sessions = KeyTeleportSessionStore::new(data_dir);
    let runtime_handle = runtime.handle().clone();
    let startup_error = Arc::new(Mutex::new(None));
    let startup_error_writer = Arc::clone(&startup_error);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);

            let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
            let repository = Arc::clone(&repository);
            let runtime_handle = runtime_handle.clone();
            let teleport_sessions = teleport_sessions.clone();
            if let Err(error) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui_kit::TitlebarOptions {
                        title: Some("Signer Lab".into()),
                        appears_transparent: true,
                        ..TitleBar::title_bar_options()
                    }),
                    window_background: WindowBackgroundAppearance::Blurred,
                    window_min_size: Some(size(px(960.0), px(640.0))),
                    ..TitleBar::window_options()
                },
                cx,
                move |window, cx| {
                    Theme::sync_system_appearance(Some(window), cx);
                    Theme::update(cx, design::apply_brand);
                    cx.activate(false);
                    cx.new(|cx| {
                        SignerLabApp::new(
                            repository,
                            runtime_handle,
                            signers,
                            teleport_sessions,
                            window,
                            cx,
                        )
                    })
                },
            ) {
                if let Ok(mut startup_error) = startup_error_writer.lock() {
                    *startup_error = Some(error.to_string());
                }

                cx.quit();
            }
        });

    let startup_error = startup_error
        .lock()
        .map_err(|_| eyre!("startup error state is unavailable"))?
        .take();
    if let Some(error) = startup_error {
        return Err(eyre!("failed to open Signer Lab window: {error}"));
    }
    Ok(())
}
