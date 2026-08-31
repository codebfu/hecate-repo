# hecate-repo

Signed feature repository management CLI

## Official repository signing key

Published artifacts at `https://repo.hecate-mcp.com` are signed with Ed25519. Operators and Hecate
servers must trust this **public** key (standard base64, raw 32-byte Ed25519):

```text
kHWEtm3yvH9wV2PPb2FMB9XJ0oM68CvUXTUxzAWeGTo=
```

Configure Hecate with:

```env
HECATE_REPO_URL=https://repo.hecate-mcp.com
RELEASE_SIGNING_PUBLIC_KEY_B64=kHWEtm3yvH9wV2PPb2FMB9XJ0oM68CvUXTUxzAWeGTo=
```

Print the key from a locally initialized repo: `hecate-repo pubkey`

`features.json` must include `generated_at` (RFC3339). Hecate 1.0.0 rejects indexes without it.

Every **hecate-repo** GitHub release (and manual `Release (binaries)` → workflow_dispatch)
builds the Linux CLI and runs `ci/republish-index-ftp.sh` so prod `dists/` is regenerated with
the new binary immediately. Manual local path:

```bash
hecate-repo reindex --repo ./repository
# or upload dists/ only:
bash ci/republish-index-ftp.sh
```

The matching **private** key lives only in CI (`hecate_REPO_SIGNING_KEY_B64` GitHub secret) and
operator backup storage — never commit it.

## License

GPL-3.0-or-later — Copyright (C) 2026 Gaultier HUBERT.

See [LICENSE](LICENSE) and the [Hecate ecosystem index](https://github.com/codebfu/hecate/blob/master/docs/ecosystem.md).

## FTP publish secrets

GitHub Actions on **this repository** (release `reindex` job) and lampad publish workflows need:

| Name | Type |
|---|---|
| `HECATE_REPO_SIGNING_KEY_B64` | secret |
| `HECATE_REPO_FTP_USER` | secret |
| `HECATE_REPO_FTP_PASSWORD` | secret |
| `HECATE_REPO_FTP_HOST` | variable |
| `HECATE_REPO_FTP_REMOTE_DIR` | variable |
| `HECATE_REPO_FTP_PROTOCOL` | variable |
| `HECATE_REPO_FTP_PORT` | variable |
| `HECATE_REPO_FTP_USE_TLS` | variable (optional) |

See `ci/publish-to-repo-ftp.sh`, `ci/republish-index-ftp.sh`, and `scripts/prepare-repo-ftp.sh`.
Release workflow job `reindex` republishes `dists/` after each CLI release.
