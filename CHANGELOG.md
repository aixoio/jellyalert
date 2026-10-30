# Changelog

Jelly Alert uses `0.x.x` release versions and will remain below `1.0.0`.

## 0.1.5 — 2026-10-30

- Reject stale notification claims when a scan changes a plan, including its episode coverage.
- Deliver specials as episode alerts in both notification modes, with matching activity and progress views.
- Preserve the legacy global mode during upgrades without altering previously published migrations.
- Bound test-send processing and align frontend deadlines; report ambiguous delivery outcomes without encouraging blind retries.
- Refresh open notification previews, close entries that leave the current view, and block tests when refreshes fail.
- Show `@everyone` in previews and explain that test sends also mention everyone.
- Open delivery history directly from service warnings and preserve activity tabs in the URL.
- Retry failed artwork while the dashboard is visible.
- Align backend and frontend package versions with the release tag.

If a previous upgrade already discarded the old global preference, check the default in Settings. This patch preserves current settings and history; it cannot reconstruct a preference that was previously deleted.
