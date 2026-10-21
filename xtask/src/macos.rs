use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::process::CommandExt,
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
const BUNDLE_IDENTIFIER: &str = "org.bitcoinppl.signer-lab";
const CLI_BINARY_NAME: &str = "signer-lab-cli";
const CLI_IDENTIFIER: &str = "org.bitcoinppl.signer-lab.cli";
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

    fn action(&self, subject: &str) -> String {
        match self {
            Self::Certificate(_) => format!("sign {subject}"),
            Self::AdHoc => format!("ad-hoc sign {subject}"),
        }
    }
}

#[derive(Clone, Copy)]
enum Profile {
    Debug,
    Release,
}

impl Profile {
    const fn directory_name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
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

    build(&repository, "signer-lab-gpui", Profile::Release)?;
    create_bundle(&repository, &staging_bundle)?;
    sign(
        &staging_bundle,
        &signing_identity()?,
        None,
        "the application bundle",
    )?;

    let bundle = install_bundle(destination, &staging_bundle)?;

    println!("Created {}", bundle.display());

    Ok(())
}

/// Builds, signs, and replaces this process with the debug CLI
///
/// Keychain approvals for ad-hoc binaries are tied to one build's code hash, so an
/// unsigned CLI asks again after every rebuild. A certificate signature with a fixed
/// identifier keeps one "Always Allow" valid across rebuilds
pub(super) fn run_cli(args: Vec<OsString>) -> Result<()> {
    let repository = repository_root()?;

    build(&repository, CLI_BINARY_NAME, Profile::Debug)?;

    let binary = built_binary(&repository, Profile::Debug, CLI_BINARY_NAME);

    sign(
        &binary,
        &signing_identity()?,
        Some(CLI_IDENTIFIER),
        "the CLI",
    )?;

    // exec hands standard streams, signals, and the exit status straight to the CLI
    let error = Command::new(&binary).args(args).exec();

    Err(error).wrap_err_with(|| format!("run {}", binary.display()))
}

fn repository_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| eyre!("xtask manifest directory has no parent"))
}

fn build(repository: &Path, package: &str, profile: Profile) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repository)
        .args(["build", "-p", package]);

    if matches!(profile, Profile::Release) {
        command.arg("--release");
    }

    run(&mut command, &format!("build {package}"))
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
        &built_binary(repository, Profile::Release, BINARY_NAME),
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

fn built_binary(repository: &Path, profile: Profile, name: &str) -> PathBuf {
    profile_directory(
        repository,
        env::var_os("CARGO_TARGET_DIR").as_deref(),
        profile,
    )
    .join(name)
}

fn profile_directory(
    repository: &Path,
    target_override: Option<&OsStr>,
    profile: Profile,
) -> PathBuf {
    let target = target_override.map_or_else(|| repository.join("target"), PathBuf::from);

    repository.join(target).join(profile.directory_name())
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination)
        .wrap_err_with(|| format!("copy {} to {}", source.display(), destination.display()))?;

    Ok(())
}

fn sign(
    path: &Path,
    identity: &SigningIdentity,
    identifier: Option<&str>,
    subject: &str,
) -> Result<()> {
    let mut command = Command::new("codesign");
    command.args([
        "--force",
        "--sign",
        identity.codesign_value(),
        "--timestamp=none",
    ]);

    // bundles take their identifier from Info.plist, bare binaries need one so the
    // designated requirement does not depend on the linker's per-build name
    if let Some(identifier) = identifier {
        command.args(["--identifier", identifier]);
    }

    run(command.arg(path), &identity.action(subject))
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
        "No Apple Development signing identity was found. Ad-hoc signatures can cause macOS to ask for Keychain access again after each rebuild."
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
    <string>{BUNDLE_IDENTIFIER}</string>
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
    use std::{ffi::OsStr, path::Path};

    use super::{Profile, apple_development_identity, profile_directory};

    #[test]
    fn builds_use_the_configured_target_directory() {
        let repository = Path::new("/workspace/signer-lab");

        assert_eq!(
            profile_directory(repository, None, Profile::Release),
            repository.join("target/release")
        );

        assert_eq!(
            profile_directory(
                repository,
                Some(OsStr::new("target/agent-core")),
                Profile::Debug
            ),
            repository.join("target/agent-core/debug")
        );

        assert_eq!(
            profile_directory(
                repository,
                Some(OsStr::new("/build/target")),
                Profile::Release
            ),
            Path::new("/build/target/release")
        );
    }

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
