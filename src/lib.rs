//! Copyright (C) 2026 Gaultier HUBERT
//! SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PRIMARY_SIGNING_KEY_ENV: &str = "HECATE_REPO_SIGNING_KEY_B64";
pub const FALLBACK_SIGNING_KEY_ENV: &str = "HECATE_RELEASE_SIGNING_KEY_B64";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default = "default_keep_versions")]
    pub keep_versions: usize,
    #[serde(default)]
    pub keep_versions_override: BTreeMap<String, usize>,
}

fn default_channel() -> String {
    "stable".to_owned()
}

fn default_keep_versions() -> usize {
    10
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            channel: default_channel(),
            keep_versions: default_keep_versions(),
            keep_versions_override: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureKind {
    Agent,
    Helper,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureCommand {
    pub name: String,
    pub description: String,
    pub risk_level: String,
    pub input_schema: Value,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpMetadata {
    #[serde(default)]
    pub skills: Vec<Value>,
    #[serde(default)]
    pub rules: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub filename: String,
    pub os: String,
    pub arch: String,
    pub sha256: String,
    pub size: u64,
    pub installer_type: String,
    /// Canonical fleet-update signature (`v1\\n{kind}\\n…`) for older agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperMetadata {
    pub binary: String,
    pub privilege: String,
    pub socket: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub id: String,
    pub min_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureManifest {
    pub id: String,
    pub kind: FeatureKind,
    pub version: String,
    pub platforms: Vec<String>,
    pub commands: Vec<FeatureCommand>,
    #[serde(default)]
    pub mcp: McpMetadata,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    pub helper: Option<HelperMetadata>,
    #[serde(default)]
    pub depends_on: Vec<Dependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturesIndex {
    pub channel: String,
    /// RFC3339 timestamp of when this index was generated.
    pub generated_at: String,
    pub features: Vec<FeatureIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureIndexEntry {
    pub id: String,
    pub kind: FeatureKind,
    pub versions: Vec<VersionIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionIndexEntry {
    pub version: String,
    pub path: String,
    pub sha256_feature_json: String,
    pub artifacts: Vec<ArtifactIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactIndexEntry {
    pub os: String,
    pub arch: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct AddOptions {
    pub repo: PathBuf,
    pub feature_json: PathBuf,
    pub artifact: PathBuf,
    pub os: String,
    pub arch: String,
    pub installer_type: String,
    pub forced_kind: Option<FeatureKind>,
    /// Replace an existing artifact for the same OS/arch when the payload differs.
    pub replace_existing: bool,
}

pub fn signing_key_from_env() -> Result<SigningKey> {
    let encoded = env::var(PRIMARY_SIGNING_KEY_ENV)
        .or_else(|_| env::var(FALLBACK_SIGNING_KEY_ENV))
        .with_context(|| {
            format!("neither {PRIMARY_SIGNING_KEY_ENV} nor {FALLBACK_SIGNING_KEY_ENV} is set")
        })?;
    signing_key_from_base64(&encoded)
}

pub fn signing_key_from_base64(encoded: &str) -> Result<SigningKey> {
    let bytes = STANDARD
        .decode(encoded.trim())
        .context("signing key is not valid standard base64")?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow!("signing key must be 32 bytes, got {}", bytes.len()))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn verifying_key_from_base64(encoded: &str) -> Result<VerifyingKey> {
    let bytes = STANDARD
        .decode(encoded.trim())
        .context("public key is not valid standard base64")?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow!("public key must be 32 bytes, got {}", bytes.len()))?;
    VerifyingKey::from_bytes(&key).context("invalid Ed25519 public key")
}

pub fn public_key_base64(key: &SigningKey) -> String {
    STANDARD.encode(key.verifying_key().to_bytes())
}

pub fn sign_bytes(key: &SigningKey, bytes: &[u8]) -> [u8; 64] {
    key.sign(bytes).to_bytes()
}

pub fn verify_bytes(key: &VerifyingKey, bytes: &[u8], signature: &[u8]) -> Result<()> {
    let raw: [u8; 64] = signature
        .try_into()
        .map_err(|_| anyhow!("signature must contain exactly 64 bytes"))?;
    key.verify(bytes, &Signature::from_bytes(&raw))
        .context("Ed25519 signature verification failed")
}

pub fn canonical_commands_equal(left: &[FeatureCommand], right: &[FeatureCommand]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_by(|a, b| a.name.cmp(&b.name));
    right.sort_by(|a, b| a.name.cmp(&b.name));
    left == right
}

pub fn init_repo(repo: &Path, key: &SigningKey) -> Result<()> {
    if repo.exists() && fs::read_dir(repo)?.next().is_some() {
        bail!("repository directory is not empty: {}", repo.display());
    }
    fs::create_dir_all(repo)?;
    let config = RepoConfig::default();
    fs::write(
        repo.join("repo.toml"),
        "channel = \"stable\"\nkeep_versions = 10\n\n[keep_versions_override]\n# agent = 20\n",
    )?;
    fs::create_dir_all(repo.join("pool"))?;
    fs::create_dir_all(repo.join("dists").join(&config.channel))?;
    regenerate_index(repo, key)
}

pub fn add_artifact(options: &AddOptions, key: &SigningKey) -> Result<()> {
    validate_component(&options.os, "OS")?;
    validate_component(&options.arch, "architecture")?;
    if !matches!(
        options.installer_type.as_str(),
        "deb" | "msi" | "pkg" | "raw"
    ) {
        bail!("unsupported installer type: {}", options.installer_type);
    }

    let input = fs::read(&options.feature_json)
        .with_context(|| format!("failed to read {}", options.feature_json.display()))?;
    let mut incoming: FeatureManifest =
        serde_json::from_slice(&input).context("invalid feature manifest JSON")?;
    validate_component(&incoming.id, "feature id")?;
    Version::parse(&incoming.version).context("feature version is not valid semver")?;
    if let Some(kind) = options.forced_kind {
        incoming.kind = kind;
    }

    let filename = options
        .artifact
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .context("artifact path has no UTF-8 filename")?
        .to_owned();
    let artifact_bytes = fs::read(&options.artifact)
        .with_context(|| format!("failed to read {}", options.artifact.display()))?;
    let version_dir = options
        .repo
        .join("pool")
        .join(&incoming.id)
        .join(&incoming.version);
    let manifest_path = version_dir.join("feature.json");

    let mut manifest = if manifest_path.exists() {
        let existing: FeatureManifest = read_json(&manifest_path)?;
        if existing.id != incoming.id
            || existing.version != incoming.version
            || existing.kind != incoming.kind
        {
            bail!("existing feature identity or kind differs from the supplied manifest");
        }
        if !canonical_commands_equal(&existing.commands, &incoming.commands) {
            bail!("command set differs from the existing version");
        }
        existing
    } else {
        incoming.artifacts.clear();
        incoming
    };

    if let Some(existing_index) = manifest
        .artifacts
        .iter()
        .position(|artifact| artifact.os == options.os && artifact.arch == options.arch)
    {
        let incoming_sha = sha256_hex(&artifact_bytes);
        let existing = &manifest.artifacts[existing_index];
        if existing.sha256 == incoming_sha
            && existing.filename == filename
            && existing.installer_type == options.installer_type
        {
            // Idempotent re-publish: still ensure fleet update_signature is present.
            let updated = backfill_manifest_update_signatures(&mut manifest, key)?;
            if updated {
                write_signed_json(&manifest_path, &manifest, key)?;
            }
            regenerate_index(&options.repo, key)?;
            return Ok(());
        }
        if !options.replace_existing {
            bail!(
                "an artifact already exists for {} {} {} {}",
                manifest.id, manifest.version, options.os, options.arch
            );
        }
        let replaced = manifest.artifacts.remove(existing_index);
        remove_stored_artifact(&version_dir, &replaced)?;
    }

    let destination_dir = version_dir.join(&options.os).join(&options.arch);
    let destination = destination_dir.join(&filename);
    if destination.exists() {
        let existing_bytes = fs::read(&destination).with_context(|| {
            format!("failed to read existing artifact {}", destination.display())
        })?;
        if sha256_hex(&existing_bytes) == sha256_hex(&artifact_bytes) {
            regenerate_index(&options.repo, key)?;
            return Ok(());
        }
        if !options.replace_existing {
            bail!(
                "artifact destination already exists: {}",
                destination.display()
            );
        }
        fs::remove_file(&destination)?;
        let sig = signature_path(&destination);
        if sig.exists() {
            fs::remove_file(&sig)?;
        }
    }

    let sha256 = sha256_hex(&artifact_bytes);
    let update_kind = update_kind_for_feature(&manifest);
    let update_signature = Some(sign_canonical_update(
        key,
        update_kind,
        &manifest.version,
        &sha256,
    ));
    let artifact = Artifact {
        filename,
        os: options.os.clone(),
        arch: options.arch.clone(),
        sha256,
        size: artifact_bytes.len() as u64,
        installer_type: options.installer_type.clone(),
        update_signature,
    };
    manifest.artifacts.push(artifact);
    manifest
        .artifacts
        .sort_by(|a, b| (&a.os, &a.arch, &a.filename).cmp(&(&b.os, &b.arch, &b.filename)));

    fs::create_dir_all(&destination_dir)?;
    fs::write(&destination, &artifact_bytes)?;
    write_signature(&destination, key)?;
    write_signed_json(&manifest_path, &manifest, key)?;
    regenerate_index(&options.repo, key)
}

pub fn remove_version(repo: &Path, id: &str, version: &str, key: &SigningKey) -> Result<()> {
    validate_component(id, "feature id")?;
    Version::parse(version).context("version is not valid semver")?;
    let version_dir = repo.join("pool").join(id).join(version);
    if !version_dir.is_dir() {
        bail!("feature version does not exist: {id} {version}");
    }
    fs::remove_dir_all(version_dir)?;
    remove_empty_dir(&repo.join("pool").join(id))?;
    regenerate_index(repo, key)
}

pub fn prune_repo(repo: &Path, key: &SigningKey) -> Result<Vec<PathBuf>> {
    let config = read_config(repo)?;
    let pool = repo.join("pool");
    let mut removed = Vec::new();
    if pool.is_dir() {
        for feature_entry in fs::read_dir(&pool)? {
            let feature_entry = feature_entry?;
            if !feature_entry.file_type()?.is_dir() {
                continue;
            }
            let id = feature_entry.file_name().to_string_lossy().into_owned();
            let keep = config
                .keep_versions_override
                .get(&id)
                .copied()
                .unwrap_or(config.keep_versions);
            let mut versions = Vec::new();
            for version_entry in fs::read_dir(feature_entry.path())? {
                let version_entry = version_entry?;
                if version_entry.file_type()?.is_dir() {
                    let text = version_entry.file_name().to_string_lossy().into_owned();
                    let version = Version::parse(&text)
                        .with_context(|| format!("invalid semver directory: {id}/{text}"))?;
                    versions.push((version, version_entry.path()));
                }
            }
            versions.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, path) in versions.into_iter().skip(keep) {
                fs::remove_dir_all(&path)?;
                removed.push(path);
            }
            remove_empty_dir(&feature_entry.path())?;
        }
    }
    regenerate_index(repo, key)?;
    Ok(removed)
}

/// Ensure every pool feature.json artifact carries a canonical fleet `update_signature`.
///
/// Returns the number of manifests rewritten.
pub fn backfill_update_signatures(repo: &Path, key: &SigningKey) -> Result<usize> {
    let pool = repo.join("pool");
    if !pool.is_dir() {
        return Ok(0);
    }
    let mut rewritten = 0usize;
    for feature_entry in fs::read_dir(&pool)? {
        let feature_entry = feature_entry?;
        if !feature_entry.file_type()?.is_dir() {
            continue;
        }
        for version_entry in fs::read_dir(feature_entry.path())? {
            let version_entry = version_entry?;
            if !version_entry.file_type()?.is_dir() {
                continue;
            }
            let manifest_path = version_entry.path().join("feature.json");
            if !manifest_path.is_file() {
                continue;
            }
            let mut manifest: FeatureManifest = read_json(&manifest_path)?;
            if !backfill_manifest_update_signatures(&mut manifest, key)? {
                continue;
            }
            write_signed_json(&manifest_path, &manifest, key)?;
            rewritten += 1;
        }
    }
    if rewritten > 0 {
        regenerate_index(repo, key)?;
    }
    Ok(rewritten)
}

fn update_kind_for_feature(manifest: &FeatureManifest) -> &'static str {
    match (&manifest.kind, manifest.id.as_str()) {
        (FeatureKind::Agent, _) => "self_update",
        (FeatureKind::Helper, "proxmox") => "proxmox_update",
        (FeatureKind::Helper, _) => "desktop_update",
    }
}

fn backfill_manifest_update_signatures(
    manifest: &mut FeatureManifest,
    key: &SigningKey,
) -> Result<bool> {
    let kind = update_kind_for_feature(manifest);
    let mut changed = false;
    for artifact in &mut manifest.artifacts {
        let expected = sign_canonical_update(key, kind, &manifest.version, &artifact.sha256);
        if artifact.update_signature.as_deref() != Some(expected.as_str()) {
            artifact.update_signature = Some(expected);
            changed = true;
        }
    }
    Ok(changed)
}

pub fn regenerate_index(repo: &Path, key: &SigningKey) -> Result<()> {
    let config = read_config(repo)?;
    validate_component(&config.channel, "channel")?;
    let mut grouped: BTreeMap<String, Vec<(Version, FeatureManifest, String)>> = BTreeMap::new();
    let pool = repo.join("pool");
    fs::create_dir_all(&pool)?;

    for feature_entry in fs::read_dir(&pool)? {
        let feature_entry = feature_entry?;
        if !feature_entry.file_type()?.is_dir() {
            continue;
        }
        for version_entry in fs::read_dir(feature_entry.path())? {
            let version_entry = version_entry?;
            if !version_entry.file_type()?.is_dir() {
                continue;
            }
            let manifest_path = version_entry.path().join("feature.json");
            if !manifest_path.is_file() {
                bail!("missing manifest: {}", manifest_path.display());
            }
            let manifest_bytes = fs::read(&manifest_path)?;
            let manifest: FeatureManifest = serde_json::from_slice(&manifest_bytes)
                .with_context(|| format!("invalid manifest: {}", manifest_path.display()))?;
            let version = Version::parse(&manifest.version)
                .with_context(|| format!("invalid version in {}", manifest_path.display()))?;
            let expected_id = feature_entry.file_name().to_string_lossy().into_owned();
            let expected_version = version_entry.file_name().to_string_lossy().into_owned();
            if manifest.id != expected_id || manifest.version != expected_version {
                bail!("manifest identity does not match its pool path");
            }
            grouped.entry(manifest.id.clone()).or_default().push((
                version,
                manifest,
                sha256_hex(&manifest_bytes),
            ));
        }
    }

    let mut features = Vec::new();
    for (id, mut manifests) in grouped {
        manifests.sort_by(|a, b| b.0.cmp(&a.0));
        let kind = manifests[0].1.kind;
        if manifests
            .iter()
            .any(|(_, manifest, _)| manifest.kind != kind)
        {
            bail!("feature {id} changes kind across versions");
        }
        let versions = manifests
            .into_iter()
            .map(|(_, manifest, manifest_hash)| {
                let base = format!("pool/{}/{}", manifest.id, manifest.version);
                let artifacts = manifest
                    .artifacts
                    .into_iter()
                    .map(|artifact| ArtifactIndexEntry {
                        path: format!(
                            "{}/{}/{}/{}",
                            base, artifact.os, artifact.arch, artifact.filename
                        ),
                        os: artifact.os,
                        arch: artifact.arch,
                        filename: artifact.filename,
                        sha256: artifact.sha256,
                        size: artifact.size,
                    })
                    .collect();
                VersionIndexEntry {
                    version: manifest.version,
                    path: base,
                    sha256_feature_json: manifest_hash,
                    artifacts,
                }
            })
            .collect();
        features.push(FeatureIndexEntry { id, kind, versions });
    }

    let index = FeaturesIndex {
        channel: config.channel.clone(),
        generated_at: Utc::now().to_rfc3339(),
        features,
    };
    let dist = repo.join("dists").join(&config.channel);
    fs::create_dir_all(&dist)?;
    let index_path = dist.join("features.json");
    write_signed_json(&index_path, &index, key)?;
    let index_bytes = fs::read(&index_path)?;
    let release = format!(
        concat!(
            "Origin: Hecate\n",
            "Label: Hecate Feature Repository\n",
            "Suite: {channel}\n",
            "Codename: {channel}\n",
            "Date: {date}\n",
            "Architectures: all\n",
            "Components: main\n",
            "SHA256:\n",
            " {hash} {size} features.json\n"
        ),
        channel = config.channel,
        date = Utc::now().to_rfc2822(),
        hash = sha256_hex(&index_bytes),
        size = index_bytes.len(),
    );
    let release_path = dist.join("Release");
    fs::write(&release_path, release.as_bytes())?;
    write_signature(&release_path, key)
}

pub fn verify_repo(repo: &Path, key: &VerifyingKey) -> Result<usize> {
    let mut files = Vec::new();
    collect_files(&repo.join("dists"), &mut files)?;
    collect_files(&repo.join("pool"), &mut files)?;
    let file_set: HashSet<PathBuf> = files.iter().cloned().collect();
    let mut verified = 0;

    for path in &files {
        if path.extension().and_then(|ext| ext.to_str()) == Some("sig") {
            let target = path.with_extension("");
            if !file_set.contains(&target) {
                bail!("orphan signature: {}", path.display());
            }
            continue;
        }
        let signature_path = signature_path(path);
        if !signature_path.is_file() {
            bail!("missing signature: {}", signature_path.display());
        }
        let bytes = fs::read(path)?;
        let signature = fs::read(&signature_path)?;
        verify_bytes(key, &bytes, &signature)
            .with_context(|| format!("invalid signature: {}", path.display()))?;
        verified += 1;
    }
    Ok(verified)
}

fn read_config(repo: &Path) -> Result<RepoConfig> {
    let path = repo.join("repo.toml");
    let contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&contents).context("invalid repo.toml")
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON: {}", path.display()))
}

fn write_signed_json<T: Serialize>(path: &Path, value: &T, key: &SigningKey) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    write_signature(path, key)
}

fn remove_stored_artifact(version_dir: &Path, artifact: &Artifact) -> Result<()> {
    let path = version_dir
        .join(&artifact.os)
        .join(&artifact.arch)
        .join(&artifact.filename);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    let sig = signature_path(&path);
    if sig.exists() {
        fs::remove_file(&sig)?;
    }
    Ok(())
}

fn write_signature(path: &Path, key: &SigningKey) -> Result<()> {
    let bytes = fs::read(path)?;
    fs::write(signature_path(path), sign_bytes(key, &bytes))?;
    Ok(())
}

fn sign_canonical_update(key: &SigningKey, kind: &str, version: &str, sha256: &str) -> String {
    let canonical = format!("v1\n{kind}\n{version}\n{sha256}\n{sha256}");
    STANDARD.encode(sign_bytes(key, canonical.as_bytes()))
}

fn signature_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_component(value: &str, description: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        bail!("invalid {description}: {value:?}");
    }
    Ok(())
}

fn remove_empty_dir(path: &Path) -> Result<()> {
    if path.is_dir() && fs::read_dir(path)?.next().is_none() {
        fs::remove_dir(path)?;
    }
    Ok(())
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), files)?;
        } else if entry.file_type()?.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn command(name: &str, description: &str) -> FeatureCommand {
        FeatureCommand {
            name: name.to_owned(),
            description: description.to_owned(),
            risk_level: "low".to_owned(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            requires_approval: false,
        }
    }

    fn write_manifest(path: &Path, commands: Vec<FeatureCommand>) {
        let manifest = FeatureManifest {
            id: "agent".to_owned(),
            kind: FeatureKind::Agent,
            version: "1.2.3".to_owned(),
            platforms: vec!["linux".to_owned(), "windows".to_owned()],
            commands,
            mcp: McpMetadata::default(),
            artifacts: Vec::new(),
            helper: None,
            depends_on: Vec::new(),
        };
        fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }

    fn setup() -> (TempDir, PathBuf, PathBuf, SigningKey) {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let manifest = temp.path().join("feature.json");
        let artifact = temp.path().join("agent.bin");
        let key = test_key();
        init_repo(&repo, &key).unwrap();
        write_manifest(&manifest, vec![command("status", "Show status")]);
        fs::write(&artifact, b"artifact").unwrap();
        (temp, repo, manifest, key)
    }

    fn add_options(repo: &Path, manifest: &Path, artifact: &Path) -> AddOptions {
        AddOptions {
            repo: repo.to_owned(),
            feature_json: manifest.to_owned(),
            artifact: artifact.to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            installer_type: "raw".to_owned(),
            forced_kind: None,
            replace_existing: false,
        }
    }

    fn add_options_with_replace(
        repo: &Path,
        manifest: &Path,
        artifact: &Path,
        replace_existing: bool,
    ) -> AddOptions {
        AddOptions {
            replace_existing,
            ..add_options(repo, manifest, artifact)
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = test_key();
        let content = b"signed content";
        let signature = sign_bytes(&key, content);
        verify_bytes(&key.verifying_key(), content, &signature).unwrap();
        assert!(verify_bytes(&key.verifying_key(), b"changed", &signature).is_err());
    }

    #[test]
    fn command_set_equality_is_order_independent_and_structural() {
        let first = vec![command("zeta", "Z"), command("alpha", "A")];
        let second = vec![command("alpha", "A"), command("zeta", "Z")];
        assert!(canonical_commands_equal(&first, &second));

        let changed = vec![command("alpha", "different"), command("zeta", "Z")];
        assert!(!canonical_commands_equal(&first, &changed));
    }

    #[test]
    fn refuses_duplicate_os_and_arch() {
        let (_temp, repo, manifest, key) = setup();
        let first_artifact = manifest.with_file_name("first.bin");
        let second_artifact = manifest.with_file_name("second.bin");
        fs::write(&first_artifact, b"first").unwrap();
        fs::write(&second_artifact, b"second").unwrap();

        add_artifact(&add_options(&repo, &manifest, &first_artifact), &key).unwrap();
        let error = add_artifact(&add_options(&repo, &manifest, &second_artifact), &key)
            .unwrap_err()
            .to_string();
        assert!(error.contains("already exists"), "{error}");
    }

    #[test]
    fn replace_existing_artifact_for_same_os_and_arch() {
        let (_temp, repo, manifest, key) = setup();
        let first_artifact = manifest.with_file_name("first.bin");
        let second_artifact = manifest.with_file_name("second.bin");
        fs::write(&first_artifact, b"first").unwrap();
        fs::write(&second_artifact, b"second").unwrap();

        add_artifact(&add_options(&repo, &manifest, &first_artifact), &key).unwrap();
        add_artifact(
            &add_options_with_replace(&repo, &manifest, &second_artifact, true),
            &key,
        )
        .unwrap();

        let stored: FeatureManifest = serde_json::from_slice(
            &fs::read(repo.join("pool/agent/1.2.3/feature.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stored.artifacts.len(), 1);
        assert_eq!(stored.artifacts[0].sha256, sha256_hex(b"second"));
    }

    #[test]
    fn allows_identical_os_and_arch_repost() {
        let (_temp, repo, manifest, key) = setup();
        let artifact = manifest.with_file_name("same.bin");
        fs::write(&artifact, b"same-bytes").unwrap();

        add_artifact(&add_options(&repo, &manifest, &artifact), &key).unwrap();
        add_artifact(&add_options(&repo, &manifest, &artifact), &key).unwrap();
    }

    #[test]
    fn refuses_different_commands_on_second_add() {
        let (_temp, repo, manifest, key) = setup();
        let linux_artifact = manifest.with_file_name("linux.bin");
        let windows_artifact = manifest.with_file_name("windows.bin");
        fs::write(&linux_artifact, b"linux").unwrap();
        fs::write(&windows_artifact, b"windows").unwrap();
        add_artifact(&add_options(&repo, &manifest, &linux_artifact), &key).unwrap();

        write_manifest(&manifest, vec![command("status", "Changed description")]);
        let mut options = add_options(&repo, &manifest, &windows_artifact);
        options.os = "windows".to_owned();
        let error = add_artifact(&options, &key).unwrap_err().to_string();
        assert!(error.contains("command set differs"), "{error}");
    }
}
