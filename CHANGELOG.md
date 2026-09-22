# Changelog

## 1.0.1 — 2026-09-22

- Publish lock: only break a presumed-stale `.publish.lock` on the final wait attempt (~10 minutes), not after 30s (avoids concurrent feature.json clobber).
- `add`: if the installer blob already exists with the same hash, still restore/update the `feature.json` artifacts entry (repairs raced publishes).

## 1.0.0 — 2026-08-31

Initial public release.

- Add `--replace` to `add` commands so CI can republish an existing version.
- Harden release workflow dispatch and asset upload retries.
