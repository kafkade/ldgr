# ADR-009: Plugin/Extension Architecture for Community Features

**Status**: Proposed
**Date**: 2026-07-03
**Decision makers**: @kafkade

## Context

Phase 6 of the roadmap ("Polish & Ecosystem") lists a **plugin/extension
architecture for community features** as a deliverable. It was never designed or
tracked, so this ADR proposes the model before any implementation begins.

Today the only general-purpose extension point is the **market data provider
registry** (`crates/ldgr-core/src/market/registry.rs`, the `QuoteProvider` trait
in `crates/ldgr-core/src/market/types.rs`, `docs/provider-development-guide.md`,
and `examples/ldgr-provider-example/`). That surface is intentionally narrow — it
covers market data only. A `git grep -i plugin` over `crates/` returns nothing:
there is no general extension mechanism.

Designing one is not straightforward because ldgr has three hard constraints that
most "plugin systems" ignore:

1. **Zero-knowledge.** The server and any sync transport only ever see encrypted
   blobs. Plaintext financial data exists only on-device, briefly, after the
   vault is unlocked. Any extension surface must not become a side channel that
   leaks plaintext vault data.
2. **Three very different platform targets.** ldgr ships to CLI (native Rust),
   iOS/iPadOS/watchOS (UniFFI → XCFramework), and web (wasm-bindgen → WASM). A
   "plugin" means something different on each, and **dynamic native code loading
   is impossible on iOS and impractical for the WASM target.**
3. **A hard WASM bundle budget.** ADR-005 fixes the `core` WASM feature at **2 MB
   compressed**. Any extension mechanism that pulls a runtime, interpreter, or
   sandbox into that bundle is a non-starter.

The existing `QuoteProvider` trait already encodes the right instincts for this
environment: it is **I/O-free** (builds URLs and parses bytes; platform code does
the HTTP), compiles to every target, and is testable without network access. The
question this ADR answers is: *how do we generalize that pattern into a coherent
extension architecture across domains, without breaking the three constraints
above?*

There are also several **existing config-driven surfaces** that already behave
like declarative extensions but were never unified or documented as such:

- CSV import profiles (`crates/ldgr-core/src/import/profile.rs`)
- Import / payee categorization rules (`crates/ldgr-core/src/import/rules.rs`)
- CLI and web theming (Phase 6 deliverable, config-driven)
- Export targets (`crates/ldgr-core/src/export/` — hledger, CSV, JSON)

## Options considered

### Option 1 — Compile-time, trait-based extensions

Generalize the `QuoteProvider` + `ProviderRegistry` pattern to other domains.
Each extensible domain defines an **I/O-free trait**; community extensions are
Rust crates that implement the trait and are **compiled into a build** (the same
model as `examples/ldgr-provider-example/`).

- **Portable**: compiles to CLI, iOS (via UniFFI), and WASM identically.
- **No sandbox concerns**: extensions are ordinary Rust, subject to the same
  review as core code; there is no downloaded/executed foreign code.
- **No bundle impact beyond the code itself**; gateable behind Cargo features.
- **Cost**: extensions require a recompile to add; not "install at runtime for
  end users." This matches the market provider model, which is acceptable for a
  security-sensitive finance app.

### Option 2 — Runtime WASM-component / sandboxed plugins

Load `.wasm` plugin modules at runtime through a component model + host bindings
(WASI-style), sandboxed away from vault internals.

- **Flexible**: end users could install plugins without recompiling.
- **Heavy and platform-fragmented**:
  - **iOS**: App Store rules forbid downloading and executing new
    executable code; a bundled WASM interpreter is fragile and rejection-prone.
  - **Web**: the app itself *is* WASM; running WASM-in-WASM (nested runtime) is
    impractical and would blow the 2 MB budget many times over.
  - **CLI**: technically feasible, but a CLI-only plugin format fragments the
    ecosystem and still requires a host interpreter dependency.
- **Security burden**: a capability/permission model, host-call auditing, and a
  supply-chain story for third-party binaries — large surface for a niche app.

### Option 3 — Declarative / config extensions

Extensions are **pure data** (rules, templates, themes, import profiles,
formatter configs) — no code execution. Several such surfaces already exist but
are ad-hoc; this option unifies them into a documented, versioned, shareable
format.

- **Safe on every platform**: data can't execute, so no sandbox and no bundle
  cost; identical on CLI/iOS/web.
- **Great fit for the "community sharing" use case**: users share a bank import
  profile, a categorization ruleset, or a theme as a small file.
- **Limited power**: can't express new computation, only configure existing
  behavior.

## Decision

**Adopt a layered extension model: compile-time trait-based extensions (Option 1)
and declarative/config extensions (Option 3) as the two primary mechanisms.
Reject/defer runtime WASM-sandboxed plugins (Option 2).**

The two accepted layers are complementary: trait extensions add *new behavior*
(new providers, formatters, export targets) at compile time; declarative
extensions *configure* existing behavior (rules, profiles, themes, templates) at
runtime and are trivially shareable. Neither introduces a foreign code runtime,
so both respect the WASM budget, the iOS sandbox, and the zero-knowledge model.

### Layer A — Trait-based extension surfaces

Generalize the market registry pattern. Each extensible domain provides:

- an **I/O-free trait** (like `QuoteProvider`) — no `reqwest`, no filesystem, no
  platform APIs; it transforms inputs to outputs and, where external data is
  needed, returns a URL/request descriptor for platform code to fetch;
- a **metadata struct** for discovery/listing (`id`, `display_name`,
  `description`, capability declaration);
- a **registry** with `register` / `get_by_id` / `list_all` and a
  `default_registry()` of built-ins (mirroring `ProviderRegistry`).

Proposed trait surfaces (market providers already exists):

| Domain | Trait (proposed) | Adds | Existing anchor |
| --- | --- | --- | --- |
| Market data | `QuoteProvider` *(shipped)* | New price sources | `market/registry.rs` |
| Report formatting | `ReportFormatter` | New report output formats | `export/`, reports |
| Export targets | `ExportTarget` | New serialization/interchange formats | `export/{hledger,csv,json}.rs` |
| Import parsing | `ImportParser` | New statement/file formats (beyond CSV/OFX) | `import/` |
| Categorization | `CategorizationRule`/`Categorizer` | Programmatic categorization strategies | `import/rules.rs` |

All trait surfaces obey the **same I/O-free contract** as `QuoteProvider` so they
compile to every platform and stay unit-testable without I/O.

### Layer B — Declarative/config extension format

Unify existing config-driven surfaces under one documented, **versioned**,
shareable schema family (serde-serializable, already true for
`CsvProfile`/`ImportRule`). A declarative extension is a small file a user can
export, share, and import:

| Kind | Existing anchor | Shareable unit |
| --- | --- | --- |
| Import profiles | `import/profile.rs` (`CsvProfile`) | "Chase checking CSV mapping" |
| Categorization rulesets | `import/rules.rs` (`ImportRule`) | "My payee → account rules" |
| Themes | Phase 6 theming (config) | "Solarized dark" |
| Templates | (new) | Recurring-transaction / report templates |

Each declarative kind carries an explicit `schema_version` so older clients can
reject or migrate unknown versions rather than silently mis-parse.

### Discovery / registration

- **Trait extensions**: registered in-process via the domain registry.
  Built-ins are added by `default_registry()`; a build that bundles community
  crates calls `register(...)` at startup (exactly as market providers do).
- **Declarative extensions**: discovered as files (import/export via CLI
  commands, iOS document picker, web file input) and validated against the
  versioned schema before use.

### Versioning

- **Trait surfaces** are versioned by ldgr-core's semver. Adding a method with a
  default keeps compatibility; a breaking change bumps the major and is called
  out in the changelog. Extension crates pin a compatible `ldgr-core` range.
- **Declarative schemas** are versioned per-kind via `schema_version`. Unknown
  future versions are rejected with a clear error (never silently ignored),
  consistent with the hledger-import philosophy in ADR-002.

## Security model (zero-knowledge boundary)

The overriding rule: **an extension must never see plaintext vault data unless
explicitly granted, and must never perform I/O inside ldgr-core.**

1. **I/O-free by construction.** Every trait surface follows the `QuoteProvider`
   contract: no networking, no filesystem, no platform APIs. Extensions cannot
   exfiltrate data because they have no I/O to exfiltrate through — the platform
   host performs all I/O and decides what to hand back.
2. **Capability-scoped inputs.** Extensions receive only the specific,
   already-decrypted data the host chooses to pass for the operation (e.g. an
   `ImportCandidate`, a report row set), never a handle to the vault, keys, or
   the SQLite store. There is no ambient access to `crypto::` or `storage::`.
3. **No code execution for declarative extensions.** Layer B is pure data
   validated against a schema; it cannot run logic, so it cannot leak anything.
4. **Trust model.** Trait extensions are Rust compiled into a build and are
   therefore subject to the same code review and supply-chain scrutiny as core
   code — there is no runtime "untrusted plugin" boundary to enforce, which is
   precisely why Option 2's heavyweight sandbox is unnecessary.
5. **Keys never cross the boundary.** No extension surface exposes `MK`, `MEK`,
   `VK`, or item keys; all key types remain `Zeroize`/`ZeroizeOnDrop` internal
   to `crypto::` (per the key hierarchy in the architecture doc).

If a future extension genuinely needs external data (e.g. a new market source),
it returns a **request descriptor** (URL/params) and the platform fetches it —
mirroring the existing market-data flow (ADR-005). The extension still never
touches the network itself.

## Platform availability matrix

| Extension kind | CLI | iOS/iPadOS | Web (WASM) | Notes |
| --- | --- | --- | --- | --- |
| Trait extensions (Layer A) | ✅ compiled into binary | ✅ compiled into XCFramework | ✅ compiled into WASM (feature-flagged) | Added at build time, not by end users at runtime |
| Declarative extensions (Layer B) | ✅ files / CLI commands | ✅ document picker / share sheet | ✅ file input / import UI | Pure data, identical everywhere |
| Runtime WASM plugins (Option 2) | ❌ deferred | ❌ App Store forbids | ❌ nested WASM impractical | Rejected in this ADR |

## WASM bundle-budget implications (explicit)

- **Layer A (trait extensions)** compile to code and are gated behind Cargo
  feature flags (per ADR-005: `core`, `sync`, `import-export`, `market`, …). A
  web build includes only the extension code it actually ships, so the `core`
  feature stays within the **2 MB compressed** budget. Community trait crates
  that a given web build does not compile in add **zero** bytes.
- **Layer B (declarative extensions)** are data loaded at runtime, not code, so
  they add **nothing** to the bundle.
- **Option 2 (runtime WASM)** would require bundling a component/interpreter host
  into the WASM app — multiple times the entire 2 MB budget — and is a primary
  reason it is rejected.
- CI already fails the build if `core` WASM exceeds 2 MB compressed (ADR-005);
  this ADR adds no mechanism that could breach that gate.

## iOS-sandbox implications (explicit)

- iOS/iPadOS **prohibit downloading and executing new executable code** at
  runtime (App Store Review Guideline 2.5.2). This makes Option 2 non-viable on
  Apple platforms.
- **Layer A** works because trait extensions are compiled into the ldgr-core
  Rust static library and delivered inside the signed XCFramework — no dynamic
  loading, fully review-compliant.
- **Layer B** works because declarative extensions are inert data imported via
  the standard document picker / share sheet; the app interprets configuration,
  it does not execute foreign code.
- watchOS inherits the same rules and, per ADR-005, only exposes a read-only
  subset — declarative extensions are read there but not authored.

## Follow-up implementation issues (proposed; not yet filed)

This ADR is design-only. Once accepted, spawn implementation sub-issues. A
proposed breakdown (each is independently shippable):

1. **`feat(core): report formatter extension trait + registry`** — Define a
   `ReportFormatter` trait and `FormatterRegistry` mirroring the market provider
   pattern; refactor at least one existing report path to go through it. *(This
   is the required "reference extension category beyond market providers.")*
2. **`feat(core): export target extension trait`** — Generalize
   `export/{hledger,csv,json}.rs` behind an `ExportTarget` trait + registry so
   new interchange formats can be added like providers.
3. **`feat(core): unify declarative extension format + versioning`** — Give
   `CsvProfile`, `ImportRule`, and themes a shared `schema_version` envelope,
   with import/export/validation and unknown-version rejection.
4. **`feat(cli): share/import declarative extensions`** — CLI commands to export
   and import declarative extensions (profiles, rulesets, themes) as files.
5. **`docs: extension development guide (generalized)`** — Extend
   `docs/provider-development-guide.md` into a broader extension guide covering
   the new trait surfaces and the declarative format, with an example crate.
6. **`feat(core): categorization strategy trait`** — Promote `import/rules.rs`
   matching into a `Categorizer` trait so alternative strategies can be plugged
   in at compile time.

Each follow-up must preserve the I/O-free trait contract, the zero-knowledge
boundary, and the WASM budget described above.

## Consequences

- **Positive**: A coherent, platform-uniform extension story that reuses a
  pattern already proven by the market registry. No new runtime, no sandbox, no
  bundle risk, no new attack surface against the zero-knowledge model.
- **Positive**: Existing ad-hoc config surfaces (profiles, rules, themes) get
  unified into a documented, versioned, shareable format — a real community
  win with minimal risk.
- **Positive**: Extensions stay pure and unit-testable (no I/O), consistent with
  ADR-005's core philosophy.
- **Negative**: Trait extensions require a recompile to add — ldgr does not get
  "install a plugin at runtime" for end users. This is a deliberate trade-off
  for a security-sensitive finance app and matches the existing provider model.
- **Negative**: Power users wanting arbitrary runtime logic are not served; if a
  compelling case emerges, Option 2 can be revisited for the **CLI target only**
  as a strictly-optional, non-default capability.

## Future considerations

- **CLI-only runtime plugins**: if demand is real, a CLI-only, opt-in WASM
  plugin host could be reconsidered without affecting iOS/web — but only behind
  an explicit capability model and never in the default build.
- **Signed declarative extensions**: a lightweight signing/provenance scheme for
  community-shared rulesets/themes if a distribution registry emerges.
- **Extension marketplace/index**: a static, git-backed index of community trait
  crates and declarative extensions, kept outside the zero-knowledge boundary.

## References

- ADR-002 — hledger integration (unknown-input rejection philosophy)
- ADR-005 — Platform boundaries, WASM bundle budget, I/O-free core
- `crates/ldgr-core/src/market/{types.rs,registry.rs}` — the reference pattern
- `crates/ldgr-core/src/import/{profile.rs,rules.rs}` — existing declarative surfaces
- `crates/ldgr-core/src/export/` — export targets
- `docs/provider-development-guide.md`, `examples/ldgr-provider-example/`
