# ADR-011: Stable, Tenant-Scoped Vault Identifiers

**Status**: Accepted  
**Date**: 2026-07-27  
**Decision makers**: @kafkade  

## Context

Every vault that syncs is known to the server by a **vault identifier**. It appears in the URL
of every vault-scoped endpoint (`/api/v1/vaults/{vault_id}/batches/…`) and it is the first path
segment of every blob in every transport's blob store:

```text
{vault_id}/
  batches/{device_id}/{batch_id}.enc
  snapshots/{snapshot_id}.enc
  devices/{device_id}.json.enc
```

Up to v1.2.0 that identifier was **derived, not stored**. The CLI computed it as a djb2 hash of
the vault *directory path* — `vault_{hash:016x}` — and recomputed it on every command. The iOS
and web clients did something worse: they asked the **user to type it** into a free-text field.

Three problems follow, and they compound.

**It is guessable.** A 64-bit non-cryptographic hash of a short, highly predictable string is
trivially enumerable, and a hand-typed identifier is worse still — real users type `vault`,
`personal`, `main`, or their email address. The identifier also leaks that it is path-derived.

**It collides across accounts.** Nearly every user keeps the vault at the default path, so
nearly every user derived the *same* identifier. The identifier was a global primary key, so the
first account to register it claimed it for the whole server.

**The collision was a permanent denial of service, and it failed silently.** A second account
registering the same identifier got a `409 Conflict`, which the CLI deliberately swallowed
("idempotent — a 409 means it already does") — so `ldgr sync setup` reported success. Every
subsequent `sync push` and `sync pull` then failed with `404` from the ownership check, because
the vault existed but belonged to someone else. On any shared or multi-tenant host, one account
could permanently lock every other account out of the default identifier, and the victim had no
way to tell why.

Authorization itself was **not** broken: `require_vault_access` was already called by all nine
vault-scoped handlers, and relay offers were already bound to the creating account. There was no
cross-tenant read. What was missing was unguessable, stable, per-account identifiers — and any
test that would fail if a future handler forgot the ownership check.

### The constraint that makes this non-trivial

The obvious fix — "generate a random UUID per device" — **breaks multi-device sync**. Two
devices must map the same logical vault to the same server vault or they never converge: the
second device would create its own empty vault and silently sync into the void.

The old scheme only worked by accident. Two devices agreed *because they hashed the same default
path* — the exact mechanism that made two accounts collide. Removing the collision therefore
means replacing the convergence mechanism too, not just the identifier.

## Decision

### 1. Identifiers are random and persisted, never derived

A vault identifier is `v1_` followed by 32 hex characters — 128 bits from the platform CSPRNG,
minted by `ldgr_core::sync::generate_vault_id()`. It is written once to the vault's `sync_state`
table and read thereafter, mirroring how `device_id` already works. Nothing derives it from a
path, a name, or anything else a third party could reproduce.

Vault-key derivation (`HKDF(vault_key, "ldgr-vault-id")`) was considered and rejected. It is
elegant — it converges across devices for free — but it couples a server-visible identifier to
key material and would break under any future vault-key rotation.

### 2. The server's response is authoritative, and a taken identifier is never a conflict

`POST /api/v1/vaults` takes an **optional** `vault_id`. `ServerDb::claim_vault` resolves it under
a single connection lock, so concurrent claims cannot race:

| Request | Outcome |
| --- | --- |
| identifier already owned by this account | returned unchanged, whatever it looks like (re-running setup is idempotent) |
| identifier free and path-safe | granted |
| identifier owned by **another** account | a fresh random identifier is minted and returned |
| identifier not path-safe | a fresh random identifier is minted and returned |
| no identifier supplied | a fresh random identifier is minted and returned |

The character-set rule gates only *new* claims. Older iOS and web builds let users
type any identifier they liked, so values like `Family Vault` exist on deployed
servers; refusing to re-claim one would lock that account out of a vault it
demonstrably owns. Only the length contract (1-128 characters) is still a hard
rejection, matching what pre-ADR-011 servers enforced.

The third row is the fix. Returning a conflict is what let one account lock out another; minting
a substitute means a squatter gains nothing and a victim is never blocked. Clients persist the
identifier they receive, not the one they asked for.

### 3. Lookups stay scoped to the authenticated account

Every vault-scoped handler calls `require_vault_access`, which answers `404 Not Found` — never
`403 Forbidden` — for a vault the caller does not own, so the identifier namespace cannot be
probed for existence.

To be precise about where the security actually comes from, because it is easy to point at the
wrong thing: the properties this ADR claims rest on four mechanisms, and the schema is not one
of them.

1. Identifiers carry 128 bits of CSPRNG entropy, so they cannot be guessed or enumerated.
2. The server mints a substitute instead of returning a conflict, so squatting cannot deny
   service to anyone.
3. `require_vault_access` scopes every vault-scoped lookup to the authenticated account — this
   predates the ADR and was already correct.
4. The cross-tenant regression tests in `crates/ldgr-server/tests/tenant_isolation.rs` fail
   loudly if a future handler forgets (3).

The `UNIQUE(user_id, id)` index added alongside is **redundant belt-and-braces**, not a control:
`vaults.id` is already a global primary key, so `(user_id, id)` cannot be violated by any row
that the primary key would admit. It is kept purely as intent documentation — it states the
tenant-scoped shape of the data in the schema itself, and it is what a future composite-key
migration would need anyway. Removing it would change no behaviour.

`vaults.id` deliberately remains the **global** primary key. Relaxing it to a composite key would
let two accounts hold the same identifier, and because `blobs.path` is literally
`{vault_id}/…` that would *create* a cross-tenant read hole where none exists — closing it again
would mean re-keying `blobs` and `devices` onto an internal surrogate and rewriting every stored
blob. With 128-bit random identifiers the global namespace is collision-free by construction, so
the rewrite would buy no security.

### 4. Devices converge by adopting, not by re-deriving

A device that has already synced keeps its identifier. A device that has not **adopts** one:

- **`ldgr sync setup`** lists the account's vaults after login. None → let the server mint one.
  Exactly one → adopt it. Several (accounts are multi-vault since #296) → ask which.
- **`ldgr devices join`** already receives the vault key over the end-to-end encrypted relay. The
  key itself identifies the vault: the joiner tries one batch from each candidate vault and
  adopts the one that decrypts. No new pairing-protocol fields, and no chance of the user
  choosing wrong. It falls back to the account's only vault, and declines to guess when several
  vaults exist and none has a batch to test against.
- **iOS and web** replace the typed field with the same adopt-or-create flow.

## Consequences

### Positive

- Identifiers carry 128 bits of entropy: not guessable, not enumerable, and they leak nothing
  about where the vault lives on disk.
- Squatting is impossible. Two accounts cannot collide by construction, and even a deliberately
  copied identifier only produces a fresh vault for the copier.
- The identifier survives moving or renaming the vault directory. Under the old scheme that
  silently repointed sync at a different, empty namespace.
- It fixes a latent silent failure: the CLI used to build its blob transport from the identifier
  in `sync-config.json` but hand the push/pull bridge a freshly recomputed one. When those
  diverged, the server transport matched neither list prefix and returned an empty page, so
  `ldgr sync pull` reported "Already up to date" while remote batches existed.
- Users no longer invent identifiers, so a whole class of "I typed the wrong vault name" support
  problems disappears.

### Negative / trade-offs

- A second device can no longer join by typing the same string. It must either pair
  (`ldgr devices add` / `join`) or sign in and adopt — which is correct, since a device without
  the vault key could never read the data anyway, but it is a visible workflow change.
- Vault identifiers are opaque, so a user cannot recognise their vault at a glance. The CLI
  prints it in `ldgr sync status` and both apps show it once connected.
- `vaults.id` staying globally unique means the tenant scoping is enforced by the ownership check
  and the random namespace rather than by the primary key. This is a deliberate trade recorded in
  §3, not an oversight.

## Migration

No existing user may be orphaned from data they have already synced.

**Server.** Additive only: one guarded `CREATE UNIQUE INDEX IF NOT EXISTS idx_vaults_user_id ON
vaults(user_id, id)` in `ServerDb::migrate`. Safe on a v1.2.0 database because `vaults.id` was
already a global primary key there, so `(user_id, id)` is trivially unique and no existing row
can violate it. No table rebuild and no blob rewrite.

**Clients.** `resolve_vault_id` adopts the identifier an upgrading vault is already filed under,
first match winning:

1. the identifier stored in `sync_state`;
2. the one persisted in `sync-config.json` by a configured server transport;
3. for a configured Dropbox/WebDAV vault — which stores no identifier anywhere — the legacy
   path-derived value, recomputed **once** and then frozen;
4. otherwise a fresh random identifier.

Steps 2 and 3 are the upgrade path. Without them an upgrading user would silently get a new
identifier and lose sight of everything already uploaded. The legacy djb2 derivation survives as
a private, migration-only helper with a test pinning its exact output, so a future refactor
cannot quietly change which blobs a legacy vault adopts.

**Compatibility.** An old client keeps sending its identifier and, if free, still gets it —
unchanged behaviour. In the other direction, a new client talking to a **pre-ADR-011 server**
handles both of that server's behaviours: omitting `vault_id` trips its required-field
validation (axum's `Json` extractor rejects the body with `422`, or the handler answers `400`),
so the client retries once with a locally minted identifier; and re-claiming an identifier it
already holds returns `409 Conflict`, which the client reads as "already registered, and ours"
rather than an error — otherwise upgrading the client before the server would lock users out of
re-authenticating when their session token expires.

## Related

- ADR-003 (sync and conflict resolution), ADR-004 (data model), ADR-005 (platform boundaries),
  ADR-008 (self-hosting and account auth — this refines the multi-tenant story).
- Issues #298 (this ADR) and #296 (multi-vault accounts, which is why disambiguation is needed).
- This corresponds to finding F9 of the private security assessment.
