# Evidence Record Format

Every accepted measurement (benchmark or milestone evidence) is retained as
a file under `docs/evidence/results/` named:

```
YYYY-MM-DD_<slug>.md        # milestone / ad-hoc evidence
YYYY-MM-DD_<slug>.yml       # machine-readable benchmark result
```

Per the benchmark plan (OKF `engineering/benchmark-plan.md`), an accepted run
records **all** of:

| Field | Meaning |
|---|---|
| `commit` | full commit hash the measurement was taken at |
| `toolchain` | `rustc --version` (and node/TAURI versions for comparison apps) |
| `os` | OS + version |
| `hardware` | CPU model, RAM, display/GPU when relevant |
| `build_mode` | debug / release, relevant flags |
| `fixture` | fixture path or app used |
| `command` | exact command line to reproduce |
| `raw` | the unedited measurement output |
| `summary` | one-paragraph interpretation |

Rules:

- Numbers without these fields are *indicative*, never claims.
- No public performance claim before the M9 baseline (PRD §11).
- Comparative runs must come from matched applications on the same machine
  and OS build, measuring whole process trees, not just executable size.
- Times reported by `velqu-lab` are wall-clock and unbenchmarked (no
  isolation, no repetition statistics) — treat as smoke signals only.

Template:

```yaml
commit: <sha>
toolchain: rustc 1.96.0 (...)
os: Linux 7.0.0 x86_64
hardware: <cpu> / <ram>
build_mode: release
fixture: examples/hello
command: velqu-lab --headless --size 800x600 --frames 5 examples/hello
raw: |
  <verbatim output>
summary: |
  <interpretation>
```
