# hello

The M1 fixture: the smallest deterministic render target.

- `index.html` + `app.css` are loaded by `velqu-lab` (and by the
  `tests/visual/hello` fixture).
- M1 renders the deterministic paint probe scene derived from these inputs
  (see `crates/velqu-view/src/probe.rs`); M2 will render this document for
  real.

Run:

```bash
cargo run -p velqu-lab -- examples/hello
cargo run -p velqu-lab -- --headless --size 800x600 examples/hello
```
