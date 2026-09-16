---
type: Backlog
title: VelquView GitHub Issue Seed
description: Initial issue set for creating and sequencing the velqu-view repository.
tags: [velqu-view, github, backlog]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Epic 0 - Repository

1. Create Rust workspace and CI.
2. Define fixture/evidence conventions.
3. Add architecture decision record for VelquView API boundary.

# Epic 1 - Renderer Spike

4. Create `velqu-shell` native window.
5. Create `velqu-view` public API.
6. Render rectangle + text.
7. Add HTML loading.
8. Add CSS/style integration.
9. Add Flexbox fixture.
10. Add Grid fixture.
11. Add scroll/clipping fixture.
12. Add image/SVG icon fixture.

# Epic 2 - Tailwind

13. Add Tailwind build fixture.
14. Define CSS Profile v0 machine-readable manifest.
15. Implement `velqu css check` prototype.
16. Build reference Tailwind dashboard.
17. Create browser/reference screenshot comparison harness.

# Epic 3 - Input

18. Pointer hit testing.
19. Focus model.
20. Keyboard events.
21. Editable text primitive.
22. Selection and clipboard.
23. IME composition.
24. Input conformance suite.

# Epic 4 - Velqu Reactive

25. Embed isolated UI QuickJS runtime.
26. Implement `vx-state`.
27. Implement `vx-text`.
28. Implement `vx-show`.
29. Implement `vx-model`.
30. Implement `@click` and input events.
31. Implement dynamic class/value/disabled bindings.
32. Add dependency invalidation graph.
33. Add static unknown-state diagnostics.

# Epic 5 - VelquView Lab

34. Local directory/app loader.
35. File watch and reload.
36. DOM inspector.
37. Style/layout inspector.
38. Reactive state inspector.
39. Event trace.
40. Frame/paint diagnostics.

# Epic 6 - Benchmarks

41. Mini IDE VelquView fixture.
42. Matched Tauri Mini IDE fixture.
43. Matched Electron Mini IDE fixture.
44. Benchmark harness.
45. Retained evidence report.
46. GO/NO-GO decision record.

# Post-GO Only

47. Design `velqu-host` capability interface.
48. Prototype RemoteApiHost.
49. Draft Velqu Desktop integration contract.
50. Decide whether to create `ther12k/velqu-desktop`.
