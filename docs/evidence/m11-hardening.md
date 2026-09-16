# M1.1 Hardening Verification Report

- Date: 2026-09-16
- Scope: API/resource hardening inserted between M1 and M2 per external
  review — [ADR 0004](../decisions/0004-m11-api-resource-hardening.md)
- Machine: 13th Gen Intel Core i5-13420H, 12 logical CPUs, 16 GiB RAM,
  Linux 7.0.0 x86_64 (Wayland + XWayland)
- Toolchain: rustc 1.96.0 (active/evidence), 1.87.0 and 1.85.0 (MSRV probes)

## What changed

1. **Source identity** (`crates/velqu-view/src/source.rs`): `SourceId`,
   `DocumentSource { id, html, base }`, `StylesheetSource { id, css }`;
   `load_stylesheet` upserts by id (hot-reload replace primitive);
   `load_html`/`load_css` remain as auto-id conveniences.
2. **Host-side asset resolution**: `AssetResolver` trait +
   `NullAssetResolver` default + `VelquView::set_asset_resolver` /
   `resolve_asset(path)`. Core performs no I/O; `velqu-lab` installs a
   traversal-rejecting directory resolver and loads sources with file-path
   identities.
3. **Viewport invariants** (`viewport.rs`): private fields,
   `Viewport::try_new` validating dimensions/scale/pixel-count
   (`MAX_PIXELS = 2^28`), `InvalidViewport` error type; painter allocation
   is fallible (`try_reserve_exact` → `VelquError::FrameAllocationFailed`),
   with u64 intermediate math (32-bit-safe).
4. **Concept-level CSS classification** (`velqu-tailwind`):
   `classify_declaration(property, value)` + `classify_at_rule(name)` →
   `Classification { compatibility, replacement }` over
   Supported/Normalized/Unsupported; value-sensitive rules for `position`,
   `display`, `transform` (3D deferred), wide-gamut colors (Normalized);
   property-level `classify_property` kept for manifest membership.
5. **Environment**: `rust-toolchain.toml` pinned to 1.96.0; CI gains an
   `msrv` lane; `actions/checkout@v7`; `LICENSE-MIT` + `LICENSE-APACHE`
   added; README licensing section points at the files.

## MSRV discovery (documented assumption corrected)

The manifest claimed `rust-version = "1.85"`. Actual probes:

```text
cargo +1.85.0 check --workspace --all-targets --locked  → FAILS
  (fontdue 0.9.4 uses `integer_sign_cast`, unstable on 1.85)
cargo +1.87.0 check --workspace --all-targets --locked  → passes
```

Manifest bumped to `rust-version = "1.87"` with the reason recorded in
Cargo.toml and ADR 0004; the CI `msrv` lane pins 1.87 so this stays true.

## Verification

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked` | **63 passed, 0 failed** (was 36) |
| Fixture hashes (1×/2×) | **unchanged** — `9f59e9fc…`, `a8e90443…`; 20/20 frames identical each |
| Window smoke (Wayland + forced X11) | presents, auto-closes on time |
| New coverage | viewport rejection paths, stylesheet upsert, base threading to resolver, null-host default, declaration/at-rule tiers |

Reproduce:

```bash
cargo test --workspace --locked
cargo +1.87.0 check --workspace --all-targets --locked   # MSRV lane
./target/release/velqu-lab --headless --size 800x600 --frames 20 examples/hello
```

## Known limitations

- `resolve_asset` has no renderer consumer yet (M2 `<img>`); the seam is
  tested at the API level in the meantime.
- Stylesheet *removal* is not exposed; only replace (upsert). Deferred until
  hot reload needs it.
- The lab's directory resolver is deliberately strict (rejects `..`,
  absolute refs); no symlink evaluation yet — fine for the dev host, worth
  revisiting if a production host appears.
- Normalized color conversion (`oklch` → sRGB) is classified but not
  implemented; that is M3 checker work.
