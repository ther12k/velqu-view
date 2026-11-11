# Transition Engine Example (ADR 0024)

Three live demonstrations of the VelquView transition engine:

1. **Hover color transition** — `transition duration-300 ease-out` on a card
   with a `:hover` rule in `<style>`. Paint-only: mid-flight frames repaint
   from cached geometry with **zero Taffy passes**.
2. **Reactive width transition** — a `:class` ternary toggles `w-64`/`w-96`
   with `transition-all duration-200 ease-linear`. Layout-affecting: each
   frame re-lays-out through Taffy at the interpolated width.
3. **Alpine-style `vx-transition`** — a `vx-show` panel with enter/leave
   phases (`opacity` + `scale`). The phase machine renders start frames,
   swaps classes, and hides on completion.

Run it:

```sh
velqu-lab --tailwind --reactive examples/transition          # native window
velqu-lab --tailwind --reactive --watch examples/transition  # with hot reload
```

Deterministic headless capture (stepped virtual clock, sequence verified
across two fresh runs):

```sh
velqu-lab --headless --tailwind --reactive --step-ms 16 --frames 20 \
  --out out/frame.png examples/transition
```

Note on the cascade: the hover card's *base* background lives in an author
rule (`.card`), not an inline style — inline declarations outrank author
rules, so a `:hover` target must not sit inline.
