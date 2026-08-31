//! Copyright (C) 2026 Gaultier HUBERT
//! SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use hecate_repo::{
    add_artifact, backfill_update_signatures, init_repo, prune_repo, public_key_base64,
    regenerate_index, remove_version, signing_key_from_env, verify_repo, verifying_key_from_base64,
    AddOptions, FeatureKind,
};

#[derive(Debug, Parser)]
#[command(name = "hecate-repo")]
#[command(about = "Manage a signed Hecate feature repository")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create an empty signed repository.
    Init { dir: PathBuf },
    /// Add an artifact described by a feature manifest.
    Add(AddArgs),
    /// Add an agent artifact, forcing the manifest kind to agent.
    AddAgent(AddArgs),
    /// Add a helper artifact, forcing the manifest kind to helper.
    AddHelper(AddArgs),
    /// Remove one complete feature version.
    RemoveVersion {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        version: String,
    },
    /// Remove old feature versions according to repo.toml.
    Prune {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Verify every served file and adjacent signature.
    Verify {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        public_key_b64: Option<String>,
    },
    /// Embed missing canonical fleet update signatures into pool feature.json files.
    BackfillUpdateSignatures {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Regenerate dists/stable (features.json + Release) from the current pool.
    Reindex {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Print the public key derived from the environment signing key.
    Pubkey,
}

#[derive(Debug, Args)]
struct AddArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    feature_json: PathBuf,
    #[arg(long)]
    artifact: PathBuf,
    #[arg(long)]
    os: String,
    #[arg(long)]
    arch: String,
    #[arg(long, value_enum, default_value_t = InstallerType::Raw)]
    installer_type: InstallerType,
    /// Replace an existing artifact for the same OS/arch when the payload differs.
    #[arg(long)]
    replace: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InstallerType {
    Deb,
    Msi,
    Pkg,
    Raw,
}

impl InstallerType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Deb => "deb",
            Self::Msi => "msi",
            Self::Pkg => "pkg",
            Self::Raw => "raw",
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { dir } => {
            let key = signing_key_from_env()?;
            init_repo(&dir, &key)?;
            println!("Initialized {}", dir.display());
        }
        Command::Add(args) => run_add(args, None)?,
        Command::AddAgent(args) => run_add(args, Some(FeatureKind::Agent))?,
        Command::AddHelper(args) => run_add(args, Some(FeatureKind::Helper))?,
        Command::RemoveVersion { repo, id, version } => {
            let key = signing_key_from_env()?;
            remove_version(&repo, &id, &version, &key)?;
            println!("Removed {id} {version}");
        }
        Command::Prune { repo } => {
            let key = signing_key_from_env()?;
            let removed = prune_repo(&repo, &key)?;
            println!("Removed {} old version(s)", removed.len());
        }
        Command::Verify {
            repo,
            public_key_b64,
        } => {
            let verifying_key = match public_key_b64 {
                Some(encoded) => verifying_key_from_base64(&encoded)?,
                None => signing_key_from_env()?.verifying_key(),
            };
            let count = verify_repo(&repo, &verifying_key)?;
            println!("Verified {count} file(s)");
        }
        Command::BackfillUpdateSignatures { repo } => {
            let key = signing_key_from_env()?;
            let rewritten = backfill_update_signatures(&repo, &key)?;
            println!("Rewrote {rewritten} feature.json file(s) with update signatures");
        }
        Command::Reindex { repo } => {
            let key = signing_key_from_env()?;
            regenerate_index(&repo, &key)?;
            println!("Regenerated features.json and Release under {}", repo.display());
        }
        Command::Pubkey => {
            println!("{}", public_key_base64(&signing_key_from_env()?));
        }
    }
    Ok(())
}

fn run_add(args: AddArgs, forced_kind: Option<FeatureKind>) -> Result<()> {
    let key = signing_key_from_env()?;
    let options = AddOptions {
        repo: args.repo,
        feature_json: args.feature_json,
        artifact: args.artifact,
        os: args.os,
        arch: args.arch,
        installer_type: args.installer_type.as_str().to_owned(),
        forced_kind,
        replace_existing: args.replace,
    };
    add_artifact(&options, &key)?;
    println!("Added artifact and regenerated repository metadata");
    Ok(())
}
