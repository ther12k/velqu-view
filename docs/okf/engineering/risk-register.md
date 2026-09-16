---
type: Risk Register
title: VelquView Risk Register
description: Principal risks, indicators, and mitigations for the renderer POC.
tags: [velqu-view, risk]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Risks

| Risk | Severity | Early Indicator | Mitigation / Decision |
|---|---:|---|---|
| Tailwind CSS gaps are too broad | High | dashboard needs many special rewrites | tighten profile, fix backend, or NO-GO |
| Text/input quality is poor | Critical | IME/caret/selection/wrapping bugs | gate before feature growth |
| Memory advantage is small | Critical | Tauri baseline is close | NO-GO or reposition |
| Renderer glue becomes browser-sized | High | repeated addition of web-only APIs | enforce non-goals |
| Reactive layer becomes another large framework | Medium | many directives/plugins/global APIs | freeze small profile |
| UI QuickJS overhead is too high | Medium | measurable startup/RSS cost | benchmark context; add native fast paths |
| Accessibility lags | High | no reliable semantic tree | AccessKit integration milestone |
| Platform divergence | High | input/render differences multiply | qualify one platform first |
| Backend types leak publicly | High | app imports Blitz/Taffy types | enforce API boundary |
| Widget SDK freezes internals too early | Medium | external API before renderer stabilizes | keep widget mechanism internal |
| AI-generated UI becomes inconsistent | Medium | many equivalent patterns | formatter/checker/canonical primitives |
| Scope drifts toward arbitrary websites | Critical | browser API requests dominate | triage against non-goals |

# Stop Conditions

Stop or substantially redesign if input/IME cannot become reliable, Tailwind Profile v0 needs pervasive hacks, benchmark advantage is too small, or maintenance trends toward full-browser complexity.
