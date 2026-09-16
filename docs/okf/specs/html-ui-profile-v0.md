---
type: Specification
title: Velqu HTML UI Profile v0
description: Initial HTML surface required for local application interfaces.
tags: [velqu-view, html, profile]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Goal

Support the HTML elements needed for application UI without promising arbitrary page compatibility.

# Required Elements

Structural:

```text
html head body main section article header footer nav aside div span
```

Text:

```text
h1-h6 p strong em code pre small label
```

Lists/tables:

```text
ul ol li table thead tbody tr th td
```

Forms:

```text
form button input textarea select option
```

Assets:

```text
img svg-subset template
```

# Deferred

No v0 guarantee for:

```text
iframe video audio canvas object embed arbitrary browser script
```

# Semantics

VelquView may parse broader HTML, but only profile elements receive compatibility guarantees.

# Accessibility

Semantic roles should be preserved where possible and exposed through AccessKit or the selected platform accessibility layer.
