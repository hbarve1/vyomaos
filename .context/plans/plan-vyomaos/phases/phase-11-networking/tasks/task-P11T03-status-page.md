# P11T03 — Status Page Content

## Phase
Phase 11 — Networking

## Goal
Flesh out the HTTP status page with live data from the 9P `/data` share: boot count, full boot log, and uptime. The page should be self-contained HTML (no external CSS/JS dependencies) and render cleanly in a browser.

## Dependencies
- P11T02 (http-server app exists)
- Phase 07 storage (boot_count.txt, boot_log.txt written by storage-demo)

## Content specification

| Endpoint | Content |
|----------|---------|
| `GET /` | Full HTML dashboard with boot count, boot log, app table |
| `GET /health` | `{"status":"ok","os":"VyomaOS","boot":N}` |
| `GET /apps` | `{"apps":[...]}` JSON array |
| `GET /log` | Plain-text boot log from `/data/boot_log.txt` |

## Notes

- The status page is generated fresh on every request (no caching), so the boot count is always current
- All data comes from `/data/` files written by `storage-demo` — no shared memory or IPC needed between apps
- Future: add a `/metrics` endpoint with response time and request count counters (stored in `/data/http_stats.txt`)
