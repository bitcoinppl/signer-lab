use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command},
};

use clap::ValueEnum;
use color_eyre::{
    Result,
    eyre::{WrapErr, bail, eyre},
};

const APP_NAME: &str = "Signer Lab.app";
const BINARY_NAME: &str = "signer-lab";
const CODESIGN_IDENTITY_ENV: &str = "SIGNER_LAB_CODESIGN_IDENTITY";

enum SigningIdentity {
    Certificate(String),
    AdHoc,
}

impl SigningIdentity {
    fn codesign_value(&self) -> &str {
        match self {
            Self::Certificate(identity) => identity,
            Self::AdHoc => "-",
        }
    }

    const fn action(&self) -> &'static str {
        match self {
            Self::Certificate(_) => "sign the application bundle",
            Self::AdHoc => "ad-hoc sign the application bundle",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum BundleDestination {
    Scratch,
    Applications,
}

impl BundleDestination {
    fn staging_path(repository: &Path) -> PathBuf {
        repository.join("_scratch/release").join(APP_NAME)
    }

    fn installation_path(self) -> Option<PathBuf> {
        match self {
            Self::Scratch => None,
            Self::Applications => Some(PathBuf::from("/Applications").join(APP_NAME)),
        }
    }
}

pub(super) fn bundle(destination: BundleDestination) -> Result<()> {
    let repository = repository_root()?;
    let staging_bundle = BundleDestination::staging_path(&repository);

    build_release(&repository)?;
    create_bundle(&repository, &staging_bundle)?;
    sign_bundle(&staging_bundle)?;

    let bundle = install_bundle(destination, &staging_bundle)?;

    println!("Created {}", bundle.display());

    Ok(())
}

fn repository_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| eyre!("xtask manifest directory has no parent"))
}

fn build_release(repository: &Path) -> Result<()> {
    run(
        Command::new("cargo").current_dir(repository).args([
            "build",
            "--release",
            "-p",
            "signer-lab-gpui",
        ]),
        "build the release application",
    )
}

fn create_bundle(repository: &Path, bundle: &Path) -> Result<()> {
    if bundle
        .try_exists()
        .wrap_err("check for an existing application bundle")?
    {
        fs::remove_dir_all(bundle)
            .wrap_err_with(|| format!("remove existing bundle at {}", bundle.display()))?;
    }

    let contents = bundle.join("Contents");
    let executables = contents.join("MacOS");
    let resources = contents.join("Resources");

    fs::create_dir_all(&executables)
        .wrap_err_with(|| format!("create {}", executables.display()))?;
    fs::create_dir_all(&resources).wrap_err_with(|| format!("create {}", resources.display()))?;

    copy(
        &repository.join("target/release").join(BINARY_NAME),
        &executables.join(BINARY_NAME),
    )?;
    copy(
        &repository.join("crates/signer-lab-gpui/assets/SignerLab.icns"),
        &resources.join("SignerLab.icns"),
    )?;
    fs::write(contents.join("Info.plist"), info_plist())
        .wrap_err("write the application Info.plist")?;

    Ok(())
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination)
        .wrap_err_with(|| format!("copy {} to {}", source.display(), destination.display()))?;

    Ok(())
}

fn sign_bundle(bundle: &Path) -> Result<()> {
    let identity = signing_identity()?;

    run(
        Command::new("codesign")
            .args([
                "--force",
                "--sign",
                identity.codesign_value(),
                "--timestamp=none",
            ])
            .arg(bundle),
        identity.action(),
    )
}

fn signing_identity() -> Result<SigningIdentity> {
    if let Some(identity) = env::var_os(CODESIGN_IDENTITY_ENV) {
        let identity = identity
            .into_string()
            .map_err(|_| eyre!("{CODESIGN_IDENTITY_ENV} is not valid Unicode"))?;
        let identity = identity.trim();
        if identity.is_empty() {
            bail!("{CODESIGN_IDENTITY_ENV} must not be empty");
        }

        return Ok(if identity == "-" {
            SigningIdentity::AdHoc
        } else {
            SigningIdentity::Certificate(identity.to_owned())
        });
    }

    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .wrap_err("list macOS code-signing identities")?;
    if output.status.success()
        && let Some(identity) = apple_development_identity(&String::from_utf8_lossy(&output.stdout))
    {
        return Ok(SigningIdentity::Certificate(identity.to_owned()));
    }

    eprintln!(
        "No Apple Development signing identity was found. The ad-hoc bundle can cause macOS to ask for Keychain access again after a rebuild."
    );
    Ok(SigningIdentity::AdHoc)
}

fn apple_development_identity(identities: &str) -> Option<&str> {
    identities.lines().find_map(|line| {
        if !line.contains("\"Apple Development:") {
            return None;
        }

        line.split_whitespace().nth(1)
    })
}

fn install_bundle(destination: BundleDestination, staged_bundle: &Path) -> Result<PathBuf> {
    let Some(installation_path) = destination.installation_path() else {
        return Ok(staged_bundle.to_path_buf());
    };

    let pending_path = temporary_sibling(&installation_path, "installing")?;
    let backup_path = temporary_sibling(&installation_path, "previous")?;

    run(
        Command::new("ditto").arg(staged_bundle).arg(&pending_path),
        "copy the staged application bundle",
    )?;

    let had_installed_bundle = installation_path
        .try_exists()
        .wrap_err("check for an existing installed application bundle")?;
    if had_installed_bundle {
        fs::rename(&installation_path, &backup_path)
            .wrap_err_with(|| format!("move the existing bundle to {}", backup_path.display()))?;
    }

    if let Err(install_error) = fs::rename(&pending_path, &installation_path) {
        if had_installed_bundle {
            fs::rename(&backup_path, &installation_path)
                .wrap_err("restore the previous application bundle after installation failed")?;
        }

        return Err(install_error)
            .wrap_err_with(|| format!("install bundle at {}", installation_path.display()));
    }

    if had_installed_bundle {
        fs::remove_dir_all(&backup_path)
            .wrap_err_with(|| format!("remove backup bundle at {}", backup_path.display()))?;
    }

    Ok(installation_path)
}

fn temporary_sibling(bundle: &Path, purpose: &str) -> Result<PathBuf> {
    let parent = bundle
        .parent()
        .ok_or_else(|| eyre!("bundle path has no parent: {}", bundle.display()))?;
    let temporary_name = format!(".{APP_NAME}.{purpose}-{}", process::id());

    Ok(parent.join(temporary_name))
}

fn run(command: &mut Command, action: &str) -> Result<()> {
    let status = command
        .status()
        .wrap_err_with(|| format!("failed to {action}"))?;

    if !status.success() {
        bail!("could not {action}: command exited with {status}");
    }

    Ok(())
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>{BINARY_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>SignerLab.icns</string>
    <key>CFBundleIdentifier</key>
    <string>org.bitcoinppl.signer-lab</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Signer Lab</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{}</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>NSCameraUsageDescription</key>
    <string>Signer Lab uses the camera to scan test wallet data and PSBT QR codes.</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#,
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    use super::apple_development_identity;

    #[test]
    fn development_identity_is_preferred_over_distribution_identities() {
        let identities = r#"
  1) D600B4CD00EAC9219B3331E4D704A455BFCBC0DA "Apple Distribution: Example"
  2) 0D43AA0F6277F4F1662140743349F510329BA6A7 "Developer ID Application: Example"
  3) B1B5AB1EF93AEB72FDD8E8289631A3AC6E4A7D57 "Apple Development: Developer"
     3 valid identities found
"#;

        assert_eq!(
            apple_development_identity(identities),
            Some("B1B5AB1EF93AEB72FDD8E8289631A3AC6E4A7D57")
        );
    }

    #[test]
    fn missing_development_identity_uses_the_caller_fallback() {
        let identities =
            "1) 0D43AA0F6277F4F1662140743349F510329BA6A7 \"Developer ID Application: Example\"";

        assert_eq!(apple_development_identity(identities), None);
    }
}
