mod app;
mod camera;
mod qr_image;

use std::sync::{Arc, Mutex};

use color_eyre::eyre::{Result, eyre};
use directories::ProjectDirs;
use gpui::{
    AppContext as _, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    px, size,
};
use gpui_component::{Root, Theme, ThemeMode};
use signer_lab_core::{KeychainSecretStore, LabStore, SignerRepository};

use crate::app::{AppRepository, SignerLabApp, design};

fn main() -> Result<()> {
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
    let repository = SignerRepository::new(store, KeychainSecretStore, data_dir.join("wallets"));
    let repository: AppRepository = Arc::new(tokio::sync::Mutex::new(repository));
    let signers = runtime.block_on(async { repository.lock().await.signers().await })?;
    let runtime_handle = runtime.handle().clone();
    let startup_error = Arc::new(Mutex::new(None));
    let startup_error_writer = Arc::clone(&startup_error);

    Application::new().run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        let theme = Theme::global_mut(cx);
        design::apply_theme(theme);

        let bounds = Bounds::centered(None, size(px(1_440.0), px(900.0)), cx);
        let repository = Arc::clone(&repository);
        let runtime_handle = runtime_handle.clone();
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Signer Lab".into()),
                    appears_transparent: false,
                    ..Default::default()
                }),
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            },
            move |window, cx| {
                cx.activate(false);
                let app =
                    cx.new(|cx| SignerLabApp::new(repository, runtime_handle, signers, window, cx));

                cx.new(|cx| Root::new(app, window, cx))
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
