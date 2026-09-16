---
type: Benchmark Specification
title: Mini IDE Benchmark
description: Capstone application used to stress serious desktop-style UI without turning the project into an IDE product.
tags: [velqu-view, benchmark, ide]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Purpose

The Mini IDE is a benchmark application, not the product. It pressures large layouts, a file-like tree, tabs, editing, shortcuts, focus, scrolling, and command-palette behavior.

# MVP Features

Required:

* explorer tree backed by fixture data;
* multiple tabs;
* editable text;
* save action can initially be local/mock;
* line numbers if practical;
* find in current document;
* command palette;
* keyboard shortcuts;
* theme toggle.

Excluded:

* VS Code extensions;
* LSP;
* debugger;
* terminal;
* Git;
* AI;
* remote development.

# Implementation Rule

Start with the generic editable text primitive. Do not create a native `<vx-editor>` merely because an IDE usually has one. Introduce a specialized native editor widget only if profiling proves the generic path insufficient.

# Example Shell

```html
<div
  vx-state="{ sidebar: true, active: 'main.rs' }"
  class="flex h-screen bg-zinc-950 text-zinc-100"
>
  <aside vx-show="sidebar" class="w-64 border-r border-zinc-800">
    <!-- fixture tree -->
  </aside>

  <main class="flex min-w-0 flex-1 flex-col">
    <nav class="flex h-10 border-b border-zinc-800">
      <button @click="active = 'main.rs'">main.rs</button>
      <button @click="active = 'app.rs'">app.rs</button>
    </nav>

    <textarea class="min-h-0 flex-1 resize-none bg-zinc-950 p-4 font-mono"></textarea>

    <footer class="h-6 border-t border-zinc-800 px-2 text-xs">Ready</footer>
  </main>
</div>
```
