---
type: Reference
title: VelquView Upstream References
description: External references relevant to the OKF bundle and renderer architecture.
tags: [velqu-view, references]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Open Knowledge Format

https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md

This bundle follows OKF v0.2 conventions: Markdown concept documents, YAML frontmatter, root `index.md`, optional `log.md`, and hierarchical directories.

# Blitz

https://github.com/DioxusLabs/blitz

Useful as renderer architecture/reference and possible feasibility backend. It is not part of VelquView's public API contract.

# Taffy

https://github.com/DioxusLabs/taffy

Candidate CSS layout engine.

# Parley

https://github.com/linebender/parley

Candidate text layout/shaping layer.

# html5ever

https://github.com/servo/html5ever

Candidate HTML parser.

# AccessKit

https://github.com/AccessKit/accesskit

Candidate accessibility bridge.

# QuickJS

https://bellard.org/quickjs/

Velqu Reactive should use an isolated frontend context with no browser/OS APIs.

# Tailwind CSS

https://tailwindcss.com/

Tier-1 CSS authoring target.

# Alpine.js

https://alpinejs.dev/

Ergonomic inspiration only. Velqu Reactive does not promise Alpine runtime/plugin compatibility.
