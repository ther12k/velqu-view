---
type: Example
title: Velqu Reactive Counter
description: Minimal example proving local reactive state without browser DOM APIs.
tags: [velqu-view, example, reactive]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Files

```text
counter/
├── index.html
└── app.css
```

# `index.html`

```html
<!doctype html>
<html>
  <body class="min-h-screen bg-zinc-950 p-8 text-zinc-100">
    <main
      vx-state="{ count: 0 }"
      class="mx-auto max-w-md rounded-xl border border-zinc-800 p-6"
    >
      <h1 class="text-xl font-semibold">Counter</h1>

      <div class="mt-6 flex items-center gap-4">
        <button @click="count--" class="rounded-md border border-zinc-700 px-3 py-2">-</button>
        <span class="min-w-12 text-center text-2xl" vx-text="count"></span>
        <button @click="count++" class="rounded-md bg-blue-600 px-3 py-2 font-medium">+</button>
      </div>
    </main>
  </body>
</html>
```

# Runtime Path

```text
click
 → Rust event targeting
 → QuickJS expression `count++`
 → state invalidation
 → `vx-text` update
 → paint
```

No backend and no network are involved.
