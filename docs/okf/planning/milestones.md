---
type: Roadmap
title: VelquView Milestones
description: Evidence-gated project roadmap for the POC and post-POC decision.
tags: [velqu-view, milestones, planning]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# V0 Roadmap

## M0 - Bootstrap

Repository, workspace, CI, decision log, fixture convention, evidence format.

Exit: reproducible local and CI build.

## M1 - Window / Paint

Native window, renderer surface, text/rectangle, resize/DPI, screenshot capture.

Exit: deterministic Hello fixture.

## M2 - HTML/CSS

HTML tree, style, block/flex/grid, text, borders/backgrounds, images, scrolling.

Exit: static application shell renders.

## M3 - Tailwind Profile

Tailwind build, compatibility checker v0, dashboard fixture, SVG icon strategy.

Exit: dashboard usable without one-off per-component patches.

## M4 - Input Gate

Pointer, keyboard, focus, editing, selection, clipboard, IME, scrolling.

Executed in slices: **M4a** input gate (hit testing, pointer/focus
events, wheel scrolling with zero relayout — ADR 0010, done); **M4b**
interaction styling (`:hover`/`:focus`/`:active` frozen to paint-only,
presentation-only repaint, cursor, focus origin — ADR 0011, done);
**M4c** editing, split internally: M4c1 editable controls (opaque
element identity, runtime input/textarea values, grapheme-safe editing,
pointer-capture selection, scroll-to-caret — ADR 0012, done), M4c2
clipboard (Copy/Cut/Paste through a host `ClipboardProvider`, arboard
confined to the shell — ADR 0013, done), M4c3 IME (preedit
presentation + atomic commit, session-scoped composition, shell-side
enablement — ADR 0014, done; interactive platform smoke pending).

Exit: forms/input fixture passes.

## M5 - Velqu Reactive

UI QuickJS runtime, v0 directives/events/bindings, state inspector.

Executed in slices: **M5a** runtime gate (bounded isolated QuickJS per
document generation — heap/stack/deadline/job budgets, deterministic
clock/random, no ambient I/O, hostile-script battery — ADR 0015,
done); **M5b** binding compiler (vx-*/:attr/@event markup lowers to a
validated Rust-owned plan — semantic checks, deterministic conflicts,
generation-scoped ids; static docs byte-identical — ADR 0016, done);
**M5c** state + events (M4 events → bounded non-reentrant
transactional turns; atomic state+UI commits, model-before-handler,
silent control writes — ADR 0017, done); **M5d**
invalidation/batching (presentation-only turn → zero Taffy passes,
structural turn → exactly one pass, no-op turn → zero repaints;
`presentation_dirty` gating plus a precise interaction-paint probe —
ADR 0018, done).

Exit: counter/forms/tabs examples pass reactive conformance — **met**:
`velqu-lab --tailwind --reactive examples/<name>` runs all three, and
`crates/velqu-view/tests/reactive_examples.rs` drives the exact files
end-to-end (events → turns → state, facts, and raster digests); see
`docs/evidence/m5-conformance.md`.

## M6 - VelquView Lab

Local app runner, reload, inspector, diagnostics.

Executed in slices (reviewer-directed): **M6a.1 event ownership**
(`pump_reactive` takes a caller-owned batch, never the queue; replay
is explicit; turn-generated events wait for the next drain — ADR 0019,
done); **M6a inspector** (ADR 0020, done) — recorded-outcome trace
(Event/Turn/Invalidation/Render records linked by monotonic ids,
attempted-vs-committed and requested-vs-completed distinctions,
coalesced bounded causes, byte- and count-bounded retention with
honest loss reporting, metadata-default capture), observational
snapshots (`&self`, cached-only, coherence/staleness exposed), and
`velqu-lab --inspect`; **M6b transactional hot reload** — candidate
builds fully (parse/compile/runtime/initial validation) before an
atomic generation swap; failures keep the old document running with
diagnostics; CSS-only reload upserts the sheet and preserves reactive
state/focus/control values/scroll (no generation reset), while
HTML/reactive reload starts a fresh generation; **M6c file watching**
— parent-directory watching (editor temp-file renames), debounced and
coalesced, content-hash suppressed (unchanged bytes → no reload).

Acceptance gate: malformed reload leaves the previous valid UI
running; fixed file swaps atomically; CSS-only reload preserves JS
state, input value, scroll, and focus; HTML reload starts a fresh
generation with stale handles/jobs rejected; five notifications for
one save → at most one reload; unchanged content → no reload;
temp-file rename detected; traces deterministic with bounded
retention; inspector observation itself causes zero layout/repaint;
reload diagnostics identify source/subsystem; reload during active
IME cancels the old composition through existing generation
semantics; M5 conformance digests unchanged without reload. End-to-end
probe: counter at 7 → CSS color edit → still 7, new color, zero
generation resets; then HTML edit → new generation, state resets per
source, old handles rejected.

Exit: layout/state failures can be diagnosed without browser tooling.

## M7 - Real Dashboard

Full reference dashboard.

Exit: Tailwind Profile v0 frozen for benchmark.

## M8 - Mini IDE

File tree, tabs, editable text, command palette, find, shortcuts, theme.

Exit: benchmark app is usable.

## M9 - Comparative Benchmark

Matched Electron/Tauri/VelquView runs.

Exit: retained evidence package.

## M10 - Decision

GO: stabilize API, design host capability layer, plan Velqu Desktop.

NO-GO: retain findings and freeze or narrow the project.
