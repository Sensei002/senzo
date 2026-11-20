# Phase 12 — Library, profiles, accounts and sync

**Status:** Implementation complete; Windows regression gate enabled  
**Desktop branch:** `development`  
**Backend repository:** [Sensei API](https://github.com/Sensei002/senzo-api)  
**UI stack:** Rust + GPUI CE  
**Storage:** SQLite, profile-scoped  
**Cloud:** Optional Supabase Auth + PostgreSQL/RLS, accessed through SENZO API

## Delivered

### Local Library and collections
- Saved movies and series, favorites, searchable saved-title view, remove/restore-safe local tombstones and profile-scoped membership.
- Collection create, rename, delete, membership add/remove and order updates.
- Library collections project into the validated Home collection/folder model.
- Saving/removing is available from the metadata details action; Library changes refresh Home projection.

### Profiles and protection
- Local profile create, rename, switch and delete (the built-in Default profile cannot be deleted).
- Profile settings use a shared active-profile scope; add-on, plugin, debrid and media-server managers resolve against the current profile rather than a hard-coded Default ID.
- Per-profile PINs use a random salt and PBKDF2-HMAC-SHA256 with constant-time comparison and temporary lockout after repeated failures.
- PIN-protected active profiles show an unlock screen on startup as well as when switching profiles.
- Legacy global settings migrate into the Default profile, while the active-profile pointer remains installation-wide.

### Offline-first sync and account lifecycle
- Every local Library, collection, collection-membership/order, profile-setting, watched-state and playback-progress mutation is committed with a durable per-profile sync-outbox entry.
- Failed pushes remain queued with exponential retry/backoff; successful pushes are acknowledged only after the API accepts the batch.
- Cloud account signup/login, refresh and logout use the optional SENZO API routes. Refresh/access credentials are stored in the OS keyring, scoped per local profile.
- Sync uses bounded batches and cursor-paged pulls. Pulled changes are validated and applied in one SQLite transaction with their cursor checkpoint, avoiding partial-page state.
- Remote conflicts use per-entity update timestamps (last-write-wins); watched markers are reconciled with newer incomplete playback progress.
- Sign-out clears the profile's local cloud session even when the network is unavailable. Local Library and playback history are retained.

### Backend
- `POST /api/account/signup`, `login`, `refresh`, `logout`.
- `POST /api/sync` with authenticated `push` and `pull` operations.
- Supabase RLS binds sync rows to `auth.uid()`; the database functions are security-invoker, and the desktop uses only the public anon key plus the signed-in user's JWT.
- Sync storage is an append-only mutation log with idempotent client mutation IDs. Each accepted change receives a new cursor, so later edits to an entity are visible to devices that have already advanced their cursor.

## Windows regression coverage

The Windows workflow runs the full workspace suite and a dedicated Phase 12 gate:
- shared ProfileScope changes are visible to all clones without cross-profile leakage;
- local Library/favorites/collections CRUD, ordering and isolation;
- profile rename/delete/default protection and settings isolation;
- PIN format, salted verification, failed-attempt lockout and successful unlock;
- outbox retry scheduling, acknowledgement and cursor persistence;
- remote sync page validation, atomic rollback, Library/profile-setting/watched reconciliation and replay conflict handling;
- cloud base URL/session validation and Home collection projection.

The same job runs formatting, Clippy, full workspace tests, the Phase 11 regression gate, native Windows release build, startup/shutdown smoke test and binary upload.

## Provisioning required to activate cloud features

The SENZO API code and SQL migration are committed to [Sensei002/senzo-api](https://github.com/Sensei002/senzo-api). The deployed service intentionally returns a clear 503 for cloud routes until Supabase is configured. To activate it, the project owner must:

1. Apply `supabase/migrations/0001_senzo_sync.sql` to the chosen Supabase project.
2. Enable the desired Supabase Auth email/password and confirmation policy.
3. Configure `SUPABASE_URL` and `SUPABASE_ANON_KEY` in the Vercel environment for the SENZO API, then deploy.

No service-role key is used by SENZO. This chat environment did not have access to the user's Supabase project/Vercel secrets, so a real account signup and cross-device sync against a provisioned production database could not be executed here. This does not affect local Library/profile/offline functionality.

## Explicit data boundaries

- Provider secrets (add-on URLs containing config tokens, plugin secrets, debrid credentials and media-server passwords) remain device-local in the OS credential store. They are never placed in cloud-sync payloads.
- Non-secret profile preferences and supported Library/watch entities are synced; credentials must be re-entered on a new device.
- Windows is the current development validation target. Linux source support remains in place; Linux CI validation is deferred as requested.
- Physical playback/output and final visual acceptance remain Phase 18 release checks.

## Gate

The Phase 12 implementation gate is covered by the targeted Windows regression suite, full workspace checks, Windows native release build and application startup/shutdown smoke test. Live cloud acceptance additionally requires the external Supabase/Vercel provisioning above.