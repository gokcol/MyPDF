use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use mypdf_milestone0::{document, identity, proof, signing, write_new};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "MyPDF milestone 0: native Rust PDF and certificate-signing proof"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Normalize an unsigned copy to the PDF structure verified for Adobe signing.
    PrepareSigning { input: PathBuf, output: PathBuf },
    /// Run the reproducible proof and independent Poppler/OpenSSL checks.
    Prove {
        #[arg(long, default_value = "artifacts/milestone0")]
        out: PathBuf,
        #[arg(long)]
        font: Option<PathBuf>,
        #[arg(long)]
        keychain: bool,
    },
    /// Generate the synthetic multi-page PDF used by the proof.
    Fixture {
        output: PathBuf,
        #[arg(long)]
        font: Option<PathBuf>,
    },
    /// Render every PDF page to PNG using Hayro (no browser or external renderer).
    Render {
        input: PathBuf,
        output_directory: PathBuf,
        #[arg(long, default_value = "1.5")]
        scale: f32,
    },
    /// Inspect page-level text runs and their operator/font identifiers.
    Text {
        input: PathBuf,
        #[arg(long, default_value = "1")]
        page: u32,
    },
    /// Replace exactly one existing complete text run, preserving other content.
    Edit {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "1")]
        page: u32,
        #[arg(long)]
        find: String,
        #[arg(long)]
        replace: String,
    },
    /// Create a self-signed personal signing identity, protected PKCS#12 and public PEM.
    CreateId {
        output: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        password_file: Option<PathBuf>,
        #[arg(long)]
        ecdsa: bool,
    },
    /// Sign with a .p12/.pfx identity. Passwords are read securely, never as arguments.
    SignP12 {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        identity: PathBuf,
        #[arg(long)]
        password_file: Option<PathBuf>,
        #[arg(long)]
        alias: Option<String>,
        #[arg(long, default_value = "Signature1")]
        field: String,
        #[arg(long)]
        invisible: bool,
    },
    /// Inspect offline integrity and trust separately; certificate is an explicit test trust anchor.
    Verify {
        input: PathBuf,
        #[arg(long)]
        trust_der: Option<PathBuf>,
    },
    #[cfg(target_os = "macos")]
    /// List certificate metadata for identities in Keychain; does not access private keys.
    KeychainList {
        #[arg(long)]
        keychain: Option<PathBuf>,
    },
    #[cfg(target_os = "macos")]
    /// Sign using an existing Keychain identity selected by its certificate SHA-256.
    SignKeychain {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        keychain: Option<PathBuf>,
        #[arg(long, default_value = "KeychainSignature")]
        field: String,
        #[arg(long)]
        invisible: bool,
    },
}

fn password(path: Option<&Path>, confirm: bool) -> Result<Zeroizing<String>> {
    let value = if let Some(path) = path {
        std::fs::read_to_string(path)?
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    } else {
        rpassword::prompt_password("Identity passphrase: ")?
    };
    let value = Zeroizing::new(value);
    if confirm && path.is_none() {
        let repeat = Zeroizing::new(rpassword::prompt_password("Confirm passphrase: ")?);
        ensure!(*value == *repeat, "passphrases do not match");
    }
    Ok(value)
}
fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Action::PrepareSigning { input, output } => {
            write_new(
                &output,
                &document::prepare_signing_copy(&std::fs::read(input)?)?,
            )?;
        }
        Action::Prove {
            out,
            font,
            keychain,
        } => {
            let font = font.map(Ok).unwrap_or_else(document::default_font)?;
            let report = proof::run(&out, &font, keychain).await?;
            println!(
                "{} checks passed. Evidence: {}",
                report.checks.len(),
                out.join("report.json").display()
            );
        }
        Action::Fixture { output, font } => {
            let font = font.map(Ok).unwrap_or_else(document::default_font)?;
            write_new(&output, &document::fixture(&font)?)?;
        }
        Action::Render {
            input,
            output_directory,
            scale,
        } => {
            print_json(&document::render(
                &std::fs::read(input)?,
                &output_directory,
                scale,
            )?)?;
        }
        Action::Text { input, page } => {
            print_json(&document::text_runs(&std::fs::read(input)?, page)?)?;
        }
        Action::Edit {
            input,
            output,
            page,
            find,
            replace,
        } => {
            write_new(
                &output,
                &document::replace_run(&std::fs::read(input)?, page, &find, &replace)?,
            )?;
        }
        Action::CreateId {
            output,
            name,
            password_file,
            ecdsa,
        } => {
            let pem = output.with_extension("pem");
            ensure!(!output.exists() && !pem.exists(), "output already exists");
            let pass = password(password_file.as_deref(), true)?;
            let created = identity::create(&name, &pass, ecdsa)?;
            write_new(&output, &created.pkcs12)?;
            write_new(&pem, created.certificate_pem.as_bytes())?;
            print_json(&identity::info(&created.certificate_der)?)?;
        }
        Action::SignP12 {
            input,
            output,
            identity: identity_path,
            password_file,
            alias,
            field,
            invisible,
        } => {
            ensure!(!output.exists(), "output already exists");
            let pass = password(password_file.as_deref(), false)?;
            let signer = identity::FileSigner::load(
                &std::fs::read(identity_path)?,
                &pass,
                alias.as_deref(),
            )?;
            let bytes = signing::sign(&std::fs::read(input)?, &signer, &field, !invisible).await?;
            write_new(&output, &bytes)?;
            print_json(&signing::verify(&bytes, None)?)?;
        }
        Action::Verify { input, trust_der } => {
            let trust = trust_der.map(std::fs::read).transpose()?;
            let report = signing::verify(&std::fs::read(input)?, trust.as_deref())?;
            print_json(&report)?;
            ensure!(!report.is_empty(), "no digital signatures found");
            ensure!(
                report.iter().all(|s| s.crypto_valid
                    && s.digest_matches
                    && s.byte_range_valid
                    && s.certificate_binding_valid),
                "signature integrity failed"
            );
        }
        #[cfg(target_os = "macos")]
        Action::KeychainList { keychain } => {
            print_json(&mypdf_milestone0::keychain::list(keychain.as_deref())?)?;
        }
        #[cfg(target_os = "macos")]
        Action::SignKeychain {
            input,
            output,
            sha256,
            keychain,
            field,
            invisible,
        } => {
            ensure!(!output.exists(), "output already exists");
            let signer = mypdf_milestone0::keychain::select(keychain.as_deref(), &sha256)
                .context("cannot select Keychain signing identity")?;
            let bytes = signing::sign(&std::fs::read(input)?, &signer, &field, !invisible).await?;
            write_new(&output, &bytes)?;
            print_json(&signing::verify(&bytes, None)?)?;
        }
    }
    Ok(())
}
