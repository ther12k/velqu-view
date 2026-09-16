# ADR 0004 — M1.1 API/resource hardening: source identity, viewport invariants, host-side assets, concept-level CSS classification

- Status: accepted (M1.1, 2026-09-16)
- Context: external review of the M0/M1 state recommended a hardening slice
  before M2 grows HTML/CSS rendering on top of the string-only loading API.

## 1. Source identity model

**Decision.** Content crossing into `velqu-view` carries identity:
`DocumentSource { id, html, base }` and `StylesheetSource { id, css }` with
host-chosen `SourceId`s. `load_stylesheet` **upserts by id** (replaces in
place, preserving cascade position). `load_html`/`load_css` remain as
auto-id conveniences so the simple API survives.

**Why before M2.** Anonymous strings lose the filesystem identity the host
already knows: M2 image resolution needs a document base, M3 diagnostics
want to name the offending stylesheet, and M6 hot reload must replace a
sheet rather than append a doppelgänger. Retrofitting identity later would
churn every call site twice.

## 2. Asset resolution is host-provided

**Decision.** `trait AssetResolver { fn resolve(AssetRequest) -> Option<Asset> }`,
installed via `VelquView::set_asset_resolver`; default `NullAssetResolver`
resolves nothing. The core renderer performs **no ambient I/O** — the
"no privileged external I/O in core" rule from the host-capabilities spec,
applied to assets. `velqu-lab` installs a directory-scoped resolver
(traversal-rejecting) as the host.

`AssetRequest { base, path }` carries the document's opaque base; the
renderer never interprets it as a filesystem path.

## 3. Viewport invariants are enforced, not documented

**Decision.** `Viewport` fields are private; construction goes through
`Viewport::try_new` which validates non-zero dimensions, finite positive
scale factor, and `width * height <= MAX_PIXELS` (2^28 px = 1 GiB RGBA8),
returning `InvalidViewport { reason }`. Frame allocation uses
`try_reserve_exact` so residual allocator failure surfaces as
`VelquError::FrameAllocationFailed` instead of an abort.

**Why.** `velqu-view` is the library boundary; the lab CLI validating args
does not protect embedders. M2 layout will spread viewport assumptions
everywhere — an invalid viewport must be unrepresentable.

## 4. CSS classification is concept-level with three tiers

**Decision.** `velqu-tailwind` classification operates on parsed CSS
concepts — `classify_declaration(property, value)` and
`classify_at_rule(name)` — returning `Classification { compatibility,
replacement }` over `Supported | Normalized | Unsupported`.
Property-name-level `classify_property` remains for manifest membership
only.

**Why.** The compatibility contract is value-sensitive: `position` is
supported while `position: sticky` is deferred; `transform` is supported
while 3D transform functions are deferred; wide-gamut colors (`oklch`,
Tailwind v4's default) are **normalized** (converted by the pipeline), not
rejected. A property-name-only classifier cannot express any of that, and
the M3 spec already requires "normalized" reporting with suggested
replacements.

## 5. Environment honesty

- Discovered while adding the MSRV CI lane: **1.85 does not compile** —
  fontdue 0.9.4 uses `integer_sign_cast`, stable since 1.87. The manifest
  now declares `rust-version = "1.87"` (verified:
  `cargo +1.87.0 check --workspace --all-targets --locked` passes;
  `cargo +1.85.0` fails inside fontdue). This corrects a documented
  assumption rather than silently keeping a false claim.
- `rust-toolchain.toml` pins 1.96.0 (the evidence toolchain) with
  rustfmt/clippy; the CI `msrv` job overrides it via `RUSTUP_TOOLCHAIN`.
- CI uses `actions/checkout@v7`; both lanes (`check` on stable, `msrv`
  on 1.87) run on every push/PR.

## 6. Licensing

`LICENSE-MIT` and `LICENSE-APACHE` added at the repository root so the
declared `MIT OR Apache-2.0` is backed by actual texts before any
contributors or distribution.

## Consequences

- Public API grows by the source/asset types and `try_new`; fixture hashes
  are unchanged (deterministic render path untouched).
- M2 can resolve images through the host seam without an API break, M3 has
  its classification model, M6 has the replace primitive.
- The `upsert` semantics make stylesheet *removal* the one obvious gap —
  deferred until hot reload actually needs it.
