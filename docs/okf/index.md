---
okf_version: "0.2"
---

# VelquView Knowledge Bundle

This bundle defines the product, architecture, MVP, implementation plan, conformance model, and project roadmap for **VelquView**: a native local HTML/Tailwind application renderer and reactive UI runtime that can later be hosted by Velqu Desktop.

## Product
* [PRD](product/prd.md) - Product requirements and success criteria.
* [MVP](product/mvp.md) - Minimum viable product and POC acceptance boundary.
* [Non-goals](product/non-goals.md) - Explicit scope exclusions that prevent VelquView from becoming a browser.

## Architecture
* [Architecture Overview](architecture/overview.md) - System model and major components.
* [Runtime Boundaries](architecture/runtime-boundaries.md) - Renderer, reactive runtime, host, and application separation.
* [Renderer Strategy](architecture/renderer-strategy.md) - Backend strategy and evidence-gated ownership.
* [Host Capabilities](architecture/host-capabilities.md) - External I/O and capability broker design.

## Specifications
* [HTML UI Profile v0](specs/html-ui-profile-v0.md) - Supported application-oriented HTML subset.
* [Tailwind/CSS Profile v0](specs/tailwind-profile-v0.md) - First-class Tailwind compatibility contract.
* [Velqu Reactive v0](specs/velqu-reactive-v0.md) - Alpine-inspired reactive markup and UI QuickJS boundary.
* [Events and Input](specs/events-input.md) - Pointer, keyboard, focus, IME, forms, and hot-path rules.

## Engineering
* [Repository Skeleton](engineering/repo-skeleton.md) - Proposed workspace layout and crate responsibilities.
* [Implementation Plan](engineering/implementation-plan.md) - Build order from renderer spike to Mini IDE.
* [Testing and Conformance](engineering/testing-conformance.md) - Golden tests, interaction tests, and compatibility suites.
* [Benchmark Plan](engineering/benchmark-plan.md) - Comparison methodology against Tauri/Electron.
* [VelquView Lab](engineering/devtools-lab.md) - Local app runner, inspector, reload, and diagnostics.
* [Risk Register](engineering/risk-register.md) - Principal engineering and product risks.

## Planning
* [Milestones](planning/milestones.md) - Evidence-gated roadmap.
* [Decision Gates](planning/decision-gates.md) - GO/NO-GO criteria.
* [GitHub Issue Seed](planning/github-issue-seed.md) - Initial issue backlog.

## Examples
* [Counter](examples/counter.md) - Minimal Velqu Reactive example.
* [Dashboard](examples/dashboard.md) - Tailwind-first application shell.
* [Mini IDE](examples/mini-ide.md) - Benchmark application definition.

## References
* [Sources](references/sources.md) - Upstream references.
* [Bundle Manifest](bundle-manifest.md) - File inventory.
* [Update Log](log.md) - Bundle history.
