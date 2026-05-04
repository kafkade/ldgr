# ADR-006: Licensing — Apache-2.0 with AGPL Server + DCO

**Status**: Accepted  
**Date**: 2026-05-03  
**Decision makers**: @kafkade  

## Context

ldgr is open-source and must balance: maximum adoption of the core library, contributor friendliness, App Store distribution compatibility, and prevention of closed-source SaaS forks of the sync server.

## Decision

| Component | License | Rationale |
|-----------|---------|-----------|
| Rust core library (`ldgr-core`) | Apache-2.0 | Rust ecosystem norm, patent grant, App Store compatible, max adoption |
| CLI (`ldgr-cli`) | Apache-2.0 | Same as core for simplicity |
| iOS/iPadOS/watchOS app | Apache-2.0 | GPL/AGPL incompatible with App Store TOS |
| Web app | Apache-2.0 | Reduces contributor friction |
| Sync server (`ldgr-server`) | AGPL-3.0 | Prevents closed-source SaaS forks of the sync server |

### Additional Protections

- **DCO (Developer Certificate of Origin)**: All contributions require `Signed-off-by` in commits (lightweight, no CLA signing ceremony).
- **Trademark**: Register "ldgr" as a trademark. Forks must use a different name.

## Evaluation

| Goal | Apache-2.0 Everywhere | Split (Apache + AGPL Server) | AGPL Everywhere |
|------|----------------------|------------------------------|-----------------|
| Prevent SaaS forks | ❌ | ✅ (server only) | ✅ |
| Maximize core adoption | ✅ | ✅ | ❌ |
| Attract contributors | ✅ | ✅ (90% Apache) | ⚠️ Many devs avoid AGPL |
| App Store compatible | ✅ | ✅ (apps are Apache) | ❌ |
| Simplicity | ✅ | ⚠️ Two licenses | ✅ |

### Why Apache-2.0 over MIT

Apache-2.0 includes an explicit patent grant, which matters for a project with significant cryptographic code. The practical difference is small, but Apache-2.0 is the safer choice.

### Why AGPL on Server Only

The sync server is the one component where SaaS exploitation is a real concern. A hosted ldgr-sync service that doesn't share source would undermine the project. AGPL on this single component is targeted and justified. Everything else is permissive to maximize adoption and contributions.

## Consequences

- Contributors face one license for 90% of the codebase (Apache-2.0)
- AGPL is isolated to the sync server — contributors who avoid AGPL can contribute to everything else
- Trademark protects the brand regardless of licensing
- DCO ensures contribution rights without CLA overhead
- The `ldgr-server` crate has its own LICENSE file (AGPL-3.0) overriding the workspace default
