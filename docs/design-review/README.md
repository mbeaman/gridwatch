# Design review provenance

How the architecture was chosen (2026-08-30): three architects wrote independent proposals from assigned angles; three judges scored them through different lenses; a synthesis merged the winner with grafts; two adversarial critics attacked it; the revision became `docs/ARCHITECTURE.md`. Kept so a future session can see *why*, not just *what*.

## Proposals

- [pragmatic-core](proposal-pragmatic-core.md) — opstui
- [plugin-first](proposal-plugin-first.md) — patchbay (working title: opsTui) — repo github.com/mbeaman/patchbay, binary `patchbay`
- [data-first](proposal-data-first.md) — opstui — "one store, many views" (crate `opstui`, repo github.com/mbeaman/opstui)

## Judge verdicts

- [maintainability](verdict-maintainability.md) — winner: patchbay — under a maintainability lens the contract (Manifest + ComponentDef + 
- [performance](verdict-performance.md) — winner: opstui — one store, many views (Proposal 3). Under a performance-and-robustness 
- [showcase](verdict-showcase.md) — winner: patchbay (contract-first workspace) — under this lens it is the only design whos

## Adversarial critiques of the synthesis

- [feasibility](critique-feasibility.md) — 7 findings
- [product](critique-product.md) — 12 findings
