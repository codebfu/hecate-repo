#!/usr/bin/env bash
# Copyright (C) 2026 Gaultier HUBERT
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail
KEY=/tmp/hecate-repo-deploy.key
cp /mnt/d/Project/perso/hecate-root/hecate-repo-deploy "$KEY"
chmod 600 "$KEY"
export RSYNC_RSH="ssh -i $KEY -o IdentitiesOnly=yes -o UserKnownHostsFile=/mnt/d/Project/perso/hecate-root/hecate-repo-known_hosts -o StrictHostKeyChecking=yes"
REPO=/mnt/d/Project/perso/hecate-root/_seed_repo/repository
rsync -a --delete "$REPO/pool/" "hecate-repo@dedi01.codebfu.fr:/var/www/hecate-repo/pool/"
rsync -a --delete --delay-updates "$REPO/dists/" "hecate-repo@dedi01.codebfu.fr:/var/www/hecate-repo/dists/"
rsync -a "$REPO/repo.toml" "hecate-repo@dedi01.codebfu.fr:/var/www/hecate-repo/repo.toml"
echo RSYNC_OK
