# starlinkmon

A lightweight macOS menu-bar app for monitoring a Starlink dish in real time.
It talks directly to the dish's local gRPC API (`192.168.100.1:9200`) — no
account login, no cloud, everything stays on your machine.

> **Status: archived / unmaintained.** I no longer use Starlink, so this
> project is no longer actively developed. It is published here as-is in case
> it is useful to someone else. Contributions are welcome but may not be
> reviewed promptly.

## Features

- **Menu-bar tray** showing connection state, live throughput (↓/↑), latency,
  and obstruction percentage at a glance, plus the three most recent outages.
- **Statistics tab** — live charts for bandwidth, latency, packet loss, and
  power draw over the last 15m / 1h / 6h / 24h ([uPlot](https://github.com/leeoniya/uPlot)).
- **Obstruction Map tab** — a 3D sky view of the dish's obstruction map
  ([three.js](https://threejs.org/)).
- **Events tab** — recent outage history with cause and duration.
- **Info tab** — device info: hardware/software version, country code,
  azimuth/elevation, GPS satellites, uptime, Ethernet link speed, SNR status.
- **Local persistence** — performance samples and outage events are stored in
  `~/Library/Application Support/com.starlinkmon.app/data.json` and pruned
  automatically (24h of performance samples, 176h of outage events).

The app closes to the tray (the window hides instead of quitting) and runs as a
macOS *accessory* app, so it stays out of the Dock.

## How it works

`starlinkmon` is a [Tauri 2](https://tauri.app/) application:

- **Backend** — Rust. Polls the dish over gRPC via [`tonic`](https://github.com/hyperium/tonic),
  using the SpaceX device proto in `proto/spacex/api/device/device.proto`.
  Status is polled every 1s; history, outages, and the obstruction map every 60s.
- **Frontend** — plain HTML/CSS/JS in `dist/` (no build step, no npm), using
  uPlot for charts and three.js for the 3D obstruction map.

The dish must be reachable at the standard local address `192.168.100.1:9200`,
i.e. the machine running the app needs to be on the Starlink LAN (or routed to
it). The address and poll intervals are compile-time constants in
`src-tauri/src/lib.rs`.

## Download a prebuilt binary

Prebuilt macOS builds are attached to the
[GitHub Releases](https://github.com/liangguo/starlinkmon/releases) page.

The provided build is for **Apple Silicon (aarch64)**. Because the app is not
code-signed or notarized, macOS Gatekeeper will block it on first launch. To
run it:

```sh
# Remove the quarantine attribute, then open normally
xattr -dr com.apple.quarantine /Applications/starlinkmon.app
open /Applications/starlinkmon.app
```

Alternatively, right-click the app and choose **Open**, then confirm in the
dialog.

## Build from source

Requirements:

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021, rustc ≥ 1.77.2)
- The [Tauri CLI](https://tauri.app/start/prerequisites/) and its platform
  prerequisites
- `protoc` is **not** required — the proto is compiled at build time via
  [`tonic-build`](https://github.com/hyperium/tonic) with its bundled compiler.

```sh
# From the repo root
cd src-tauri

# Debug build + run (plain `cargo run` / `cargo tauri dev` will NOT work here:
# the frontend assets must be embedded, and there is no frontend dev server)
cargo tauri build --debug
open target/debug/bundle/macos/starlinkmon.app

# Release bundle (produces a .app and .dmg under target/release/bundle/)
cargo tauri build
```

The frontend requires no build step — `dist/` holds prebuilt static files that
Tauri embeds into the app bundle at build time.

## Project layout

```
starlinkmon/
├── dist/                     # Frontend (HTML/CSS/JS, vendored uPlot + three.js)
│   ├── index.html
│   └── lib/
├── proto/
│   └── spacex/api/device/device.proto   # SpaceX device gRPC definitions
└── src-tauri/                # Rust / Tauri backend
    ├── src/lib.rs            # gRPC polling, persistence, tray, Tauri commands
    ├── src/main.rs
    ├── Cargo.toml
    └── tauri.conf.json
```

## Notes & credits

- The `device.proto` definitions describe SpaceX's / Starlink's local device
  API and are the property of their respective owner; they are included here
  only to build against the dish's local interface.
- Charts: [uPlot](https://github.com/leeoniya/uPlot) (MIT).
  3D view: [three.js](https://threejs.org/) (MIT).
- Built with [Tauri](https://tauri.app/).
- Portions of this project were developed with the assistance of an AI coding
  assistant.

*Starlink is a trademark of Space Exploration Technologies Corp. (SpaceX). This
project is an independent, unofficial tool and is not affiliated with, endorsed
by, or sponsored by SpaceX.*

## License

[BSD 3-Clause](./LICENSE) © Liang Guo
