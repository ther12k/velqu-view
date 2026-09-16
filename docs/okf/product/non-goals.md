---
type: Scope
title: VelquView Non-goals
description: Hard boundaries that keep VelquView an application renderer rather than a browser.
tags: [velqu-view, scope, non-goals]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Not a General Browser

VelquView does not target arbitrary public websites, navigation/history compatibility, browser cookies, Service Workers, iframes, browser extensions, or DevTools protocol compatibility.

# Not a Browser JavaScript Environment

Velqu Reactive does not provide:

```text
window
document
navigator
MutationObserver
ResizeObserver
IntersectionObserver
localStorage
sessionStorage
browser fetch
DOM mutation APIs
```

# Not a Web Media Platform

No initial commitment to HTML video/audio compatibility, DRM, WebRTC, WebAudio, Canvas, WebGL, WebGPU web API, or WebXR.

A future native widget may solve a specific need without expanding the generic browser surface.

# Not a Rust-first UI Framework

Ordinary UI remains HTML/Tailwind-first. Rust is internal and later available for advanced native widgets.

# Not Velqu Desktop

VelquView is the UI runtime. Velqu Desktop is a later host that can add native I/O, packaging, system APIs, and direct Velqu Core integration.

# Not WASM on Desktop

WASM remains a Velqu web deployment technology. It is not inserted into the native VelquView path.
