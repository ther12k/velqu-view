# tabs

The reactive tabs example: `@click` handlers switch a `tab` state key;
`:class` bindings restyle the active tab; `vx-show` toggles the panels
(one structural mutation per switch — at most one layout pass, per
ADR 0018). The counter panel combines tab switching with stateful
editing inside a hidden-able subtree.

Run: `velqu-lab --tailwind --reactive examples/tabs`.

Conformance is pinned by `crates/velqu-view/tests/reactive_examples.rs`.
