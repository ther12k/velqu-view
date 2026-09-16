---
type: Architecture
title: Host Capabilities
description: Capability-based extension model for network, native I/O, and future Velqu Desktop integration.
tags: [velqu-view, host, capabilities]
generated:
  by: openai/gpt-5.6-sol
  at: 2026-09-16T00:00:00Z
---

# Goal

VelquView should run with zero privileged external I/O and gain capabilities only from its host.

# Capability Examples

```text
network
filesystem
clipboard
dialogs
database
process
notifications
system tray
native menu
```

# Conceptual Interface

```rust
pub trait AppHost {
    fn supports(&self, capability: CapabilityKind) -> bool;
    fn request(&self, request: HostRequest) -> HostResponse;
}
```

# Host Types

## NullHost

No external I/O. Useful for prototypes, deterministic fixtures, and renderer tests.

## RemoteApiHost

Provides approved network access.

```toml
[capabilities.network]
allow = ["https://api.example.com"]
```

## Future VelquDesktopHost

Provides filesystem, clipboard, dialogs, database, process, notifications, and direct Velqu Core dispatch.

# Design Rule

Velqu Reactive expressions must not bypass the host broker. There is no direct frontend `fetch()`, filesystem API, or process API in v0.
