use gpui_kit::component::ThemeMode;
use signer_lab_gpui::test_support::{ScreenshotFixtures, screenshot_states};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

/// Renders every screenshot state in light and dark mode
///
/// `SHOTS_DIR` overrides the output directory. `SHOTS_ONLY` takes comma-separated state-name
/// prefixes, for example `SHOTS_ONLY=bulk,sign`, and renders only matching states
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::var_os("SHOTS_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_scratch/redesign/shots"),
        PathBuf::from,
    );

    let only = std::env::var("SHOTS_ONLY").ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|prefix| !prefix.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });

    std::fs::create_dir_all(&directory)?;

    let fixtures = ScreenshotFixtures::new()?;
    let mut failures = Vec::new();
    for (mode, name) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
        for state in screenshot_states() {
            let shot = format!("{}-{name}", state.name());
            if let Some(prefixes) = &only
                && !prefixes
                    .iter()
                    .any(|prefix| shot.starts_with(prefix.as_str()))
            {
                continue;
            }

            let path = directory.join(format!("{shot}.png"));
            // each state renders in its own headless app, so one broken state must not hide the rest
            match catch_unwind(AssertUnwindSafe(|| fixtures.render(state, mode, &path))) {
                Ok(Ok(())) => println!("{}", path.display()),
                Ok(Err(error)) => failures.push(format!("{shot}: {error}")),
                Err(_) => failures.push(format!("{shot}: panicked")),
            }
        }
    }

    if failures.is_empty() {
        return Ok(());
    }

    for failure in &failures {
        eprintln!("screenshot failed: {failure}");
    }
    Err(format!("{} screenshot state(s) failed", failures.len()).into())
}
