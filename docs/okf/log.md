# VelquView Bundle Update Log

## 2026-09-16

* **Creation**: Established the VelquView OKF v0.2 bundle.
* **Architecture**: Defined VelquView as an independent native local UI runtime, not a general browser.
* **Scope**: Kept WASM out of VelquView and Velqu Desktop; WASM remains for real web deployment.
* **Reactive**: Added Velqu Reactive, an Alpine-inspired constrained expression model backed by a small isolated frontend QuickJS runtime.
* **Styling**: Made Tailwind the Tier-1 styling target through compiled CSS and a Velqu CSS compatibility profile.
* **Hosting**: External I/O is provided only through explicit host capabilities such as remote API adapters or a future Velqu Desktop host.
* **Planning**: Added POC milestones, benchmark gates, repository skeleton, conformance strategy, Mini IDE benchmark, and GitHub issue seed.
