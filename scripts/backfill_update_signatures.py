#!/usr/bin/env python3
"""Backfill canonical fleet update_signature fields into a local hecate-repo tree."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
from datetime import datetime, timezone
from email.utils import format_datetime
from pathlib import Path

from nacl.signing import SigningKey

KIND_MAP = {
    ("agent", "agent"): "self_update",
    ("helper", "desktop"): "desktop_update",
    ("helper", "proxmox"): "proxmox_update",
}


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sign_bytes(key: SigningKey, data: bytes) -> bytes:
    return key.sign(data).signature


def write_sig(path: Path, key: SigningKey) -> None:
    path.with_name(path.name + ".sig").write_bytes(sign_bytes(key, path.read_bytes()))


def update_kind(manifest: dict) -> str:
    kind = manifest.get("kind", "helper")
    feature_id = manifest["id"]
    if kind == "agent":
        return "self_update"
    if feature_id == "proxmox":
        return "proxmox_update"
    return "desktop_update"


def canonical_sig(key: SigningKey, kind: str, version: str, sha256: str) -> str:
    message = f"v1\n{kind}\n{version}\n{sha256}\n{sha256}".encode()
    return base64.b64encode(sign_bytes(key, message)).decode()


def backfill_manifest(path: Path, key: SigningKey) -> bool:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    kind = update_kind(manifest)
    changed = False
    for artifact in manifest.get("artifacts", []):
        expected = canonical_sig(key, kind, manifest["version"], artifact["sha256"])
        if artifact.get("update_signature") != expected:
            artifact["update_signature"] = expected
            changed = True
    if not changed:
        return False
    payload = (json.dumps(manifest, indent=2, ensure_ascii=False) + "\n").encode()
    path.write_bytes(payload)
    write_sig(path, key)
    return True


def regenerate_index(repo: Path, key: SigningKey) -> None:
    channel = "stable"
    repo_toml = repo / "repo.toml"
    if repo_toml.is_file():
        for line in repo_toml.read_text(encoding="utf-8").splitlines():
            if line.strip().startswith("channel"):
                channel = line.split("=", 1)[1].strip().strip('"').strip("'")
                break

    grouped: dict[str, list[tuple[str, dict, str]]] = {}
    pool = repo / "pool"
    for feature_dir in sorted(p for p in pool.iterdir() if p.is_dir()):
        for version_dir in sorted(p for p in feature_dir.iterdir() if p.is_dir()):
            manifest_path = version_dir / "feature.json"
            raw = manifest_path.read_bytes()
            manifest = json.loads(raw)
            grouped.setdefault(manifest["id"], []).append(
                (manifest["version"], manifest, sha256_hex(raw))
            )

    features = []
    for feature_id, versions in grouped.items():
        versions.sort(key=lambda item: [int(p) for p in item[0].split(".")], reverse=True)
        kind = versions[0][1]["kind"]
        version_entries = []
        for version, manifest, manifest_hash in versions:
            base = f"pool/{manifest['id']}/{manifest['version']}"
            artifacts = []
            for artifact in manifest.get("artifacts", []):
                artifacts.append(
                    {
                        "os": artifact["os"],
                        "arch": artifact["arch"],
                        "filename": artifact["filename"],
                        "sha256": artifact["sha256"],
                        "size": artifact["size"],
                        "path": f"{base}/{artifact['os']}/{artifact['arch']}/{artifact['filename']}",
                    }
                )
            version_entries.append(
                {
                    "version": version,
                    "path": base,
                    "sha256_feature_json": manifest_hash,
                    "artifacts": artifacts,
                }
            )
        features.append({"id": feature_id, "kind": kind, "versions": version_entries})

    index = {"channel": channel, "features": features}
    dist = repo / "dists" / channel
    dist.mkdir(parents=True, exist_ok=True)
    index_path = dist / "features.json"
    index_path.write_bytes((json.dumps(index, indent=2, ensure_ascii=False) + "\n").encode())
    write_sig(index_path, key)

    index_bytes = index_path.read_bytes()
    release = (
        "Origin: Hecate\n"
        "Label: Hecate Feature Repository\n"
        f"Suite: {channel}\n"
        f"Codename: {channel}\n"
        f"Date: {format_datetime(datetime.now(timezone.utc))}\n"
        "Architectures: all\n"
        "Components: main\n"
        "SHA256:\n"
        f" {sha256_hex(index_bytes)} {len(index_bytes)} features.json\n"
    )
    release_path = dist / "Release"
    release_path.write_text(release, encoding="utf-8")
    write_sig(release_path, key)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--signing-key-b64", required=True)
    args = parser.parse_args()
    key = SigningKey(base64.b64decode(args.signing_key_b64.strip()))
    rewritten = 0
    for manifest_path in sorted((args.repo / "pool").glob("*/*/feature.json")):
        if backfill_manifest(manifest_path, key):
            rewritten += 1
            print(f"updated {manifest_path}")
    regenerate_index(args.repo, key)
    print(f"rewrote {rewritten} manifest(s) and regenerated index")


if __name__ == "__main__":
    main()
