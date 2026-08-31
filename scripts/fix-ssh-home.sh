#!/usr/bin/env bash
# Copyright (C) 2026 Gaultier HUBERT
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail
# Fix sshd home-dir permissions and separate home from nginx docroot.
HOME_DIR=/var/lib/hecate-repo
DOCROOT=/var/www/hecate-repo
USER_NAME=hecate-repo
PUBKEY_LINE='command="/usr/local/bin/hecate-repo-ssh",no-port-forwarding,no-X11-forwarding,no-agent-forwarding,no-pty ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJEbPjrUyL2QZoqs9A+qPWTUFQH6iRV/w5OOvxcsAGsg hecate-repo-ci-deploy'

mkdir -p "$HOME_DIR/.ssh"
chown -R "$USER_NAME:$USER_NAME" "$HOME_DIR"
chmod 755 "$HOME_DIR"
chmod 700 "$HOME_DIR/.ssh"
printf '%s\n' "$PUBKEY_LINE" > "$HOME_DIR/.ssh/authorized_keys"
chmod 600 "$HOME_DIR/.ssh/authorized_keys"
chown "$USER_NAME:$USER_NAME" "$HOME_DIR/.ssh/authorized_keys"

# Move home away from world-writable docroot
usermod -d "$HOME_DIR" "$USER_NAME"

# Docroot: owned by deploy user, not group/world writable
chown -R "$USER_NAME:$USER_NAME" "$DOCROOT"
chmod 755 "$DOCROOT"
chmod -R a+rX "$DOCROOT/dists" "$DOCROOT/pool" "$DOCROOT/repo.toml" 2>/dev/null || true
# Keep .ssh out of docroot if leftover
rm -rf "$DOCROOT/.ssh"

cat > /etc/ssh/sshd_config.d/hecate-repo.conf <<EOF
Match User $USER_NAME
  AllowTcpForwarding no
  X11Forwarding no
  PermitTTY no
  AuthorizedKeysFile $HOME_DIR/.ssh/authorized_keys
EOF

systemctl reload ssh
echo fixed_ok
