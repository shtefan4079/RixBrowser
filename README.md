# RixBrowser

<div align="center">

**A modern, modular, experimental web browser engine written in Rust.**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 1.85+](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![Platform: Linux](https://img.shields.io/badge/Platform-Linux-brightgreen.svg)]()
[![Status: Alpha](https://img.shields.io/badge/Status-Early%20Alpha-yellow.svg)]()

</div>

---

RixBrowser is an experimental web browser engine built entirely from first principles in Rust. It fetches web pages over modern protocols (HTTP/1.1, HTTP/2, and HTTP/3 via QUIC), parses HTML5 and CSS, runs JavaScript within isolated page worker processes, computes layouts with Taffy and HarfBuzz, and renders hardware-accelerated vector graphics using Vello on the GPU.

> **Status: Early Alpha.** RixBrowser is an active research and development engine. APIs, IPC protocols, and internal storage formats evolve between revisions.

---

## Architecture Overview

RixBrowser is built as a multi-process, modular workspace of focused Rust crates:

```
┌─────────────────────────────────────────────────────────────┐
│                    rix-app (Browser Shell)                  │
│       Windowing, Tab Management, Shell UI, Automation API   │
└──────────────┬──────────────────────────────┬───────────────┘
               │ Unix Domain Sockets / shmem  │
               ▼                              ▼
┌──────────────────────────────┐┌──────────────────────────────┐
│  rix-page-worker (Tab 1)     ││  rix-page-worker (Tab 2)     │
│  - DOM & HTML5 (rix-dom)     ││  - DOM & HTML5 (rix-dom)     │
│  - CSS Cascade (rix-css)     ││  - CSS Cascade (rix-css)     │
│  - JS Runtime (rix-js/jit)   ││  - JS Runtime (rix-js/jit)   │
│  - Layout Tree (rix-layout)  ││  - Layout Tree (rix-layout)  │
│  - Display List (rix-render) ││  - Display List (rix-render) │
└──────────────┬───────────────┘└──────────────┬───────────────┘
               │ Shared Memory Framebuffers    │
               ▼                               ▼
┌─────────────────────────────────────────────────────────────┐
│                   Vello GPU Rasterizer (Vulkan)             │
└─────────────────────────────────────────────────────────────┘
```

### Core Crates

| Crate | Responsibility |
|:---|:---|
| [`rix-app`](crates/rix-app) | Main application entry point, CLI parser, and process supervision. |
| [`rix-chrome`](crates/rix-chrome) | Native browser shell, tab orchestrator, built-in browser UI (`rix://chrome/browser.html`), and automation server. |
| [`rix-core`](crates/rix-core) | Common domain types, URL abstraction, profile configuration, and logging setup. |
| [`rix-css`](crates/rix-css) | CSS parser, cascade resolution, rule indexing, and computed style engine. |
| [`rix-dom`](crates/rix-dom) | HTML5 DOM tree, node indexing, Shadow DOM, Custom Elements, and mutation observation. |
| [`rix-html`](crates/rix-html) | Spec-compliant HTML tokenizer and tree builder. |
| [`rix-ipc`](crates/rix-ipc) | Inter-process communication primitives, shared memory rings, and message passing. |
| [`rix-jit`](crates/rix-jit) | Cranelift-based baseline JIT compiler with on-stack replacement. |
| [`rix-js`](crates/rix-js) | JavaScript engine (vendored Boa runtime) integrated with Web APIs, DOM bridges, and bytecode cache. |
| [`rix-layout`](crates/rix-layout) | Document layout engine backed by Taffy, handling flexbox, grid, and inline text placement. |
| [`rix-media`](crates/rix-media) | MP4/WebM demuxers, software decoders (H.264, VP8, AAC, Opus), and Media Source Extensions. |
| [`rix-net`](crates/rix-net) | Async network stack: Happy Eyeballs DNS, rustls TLS, HTTP/1.1, HTTP/2, HTTP/3, and RFC 9111 disk cache. |
| [`rix-render`](crates/rix-render) | Retained display list compiler, SVG path rendering, and Vello GPU backend. |
| [`rix-storage`](crates/rix-storage) | Embedded transactional storage powered by `redb`. |
| [`rix-text`](crates/rix-text) | Font loading, system font fallback discovery, and HarfBuzz text shaping. |

---

## Features

- **Modern Networking Stack**:
  - Full support for HTTP/1.1, HTTP/2, and HTTP/3 (QUIC).
  - RFC 9111 HTTP cache with heuristic freshness, stale-while-revalidate, and 304 Not Modified revalidation.
  - DNS resolution with Happy Eyeballs dual-stack fallback.
  - TLS 1.2 and 1.3 powered by `rustls`.
  - Spec-compliant cookie store and redirect handling.

- **Fast Layout & Graphics Pipeline**:
  - Hardware-accelerated GPU 2D rendering via [Vello](https://github.com/linebender/vello).
  - High-performance text shaping with HarfBuzz and system font matching.
  - Flexbox and CSS Grid layout algorithms using [Taffy](https://github.com/DioxusLabs/taffy).
  - Full SVG path rasterization and symbol reuse.

- **Extensible JavaScript Runtime**:
  - Full ECMAScript support powered by vendored Boa engine.
  - Comprehensive Web APIs: `fetch`, `Streams`, `AbortController`, `MutationObserver`, `IntersectionObserver`, `ResizeObserver`, `Canvas2D`, and `MediaSource`.
  - Persistent disk bytecode cache for rapid second-load startup.
  - Custom Cranelift JIT engine for hot execution paths.

- **Process Isolation**:
  - Multi-process architecture separating chrome UI and page execution.
  - Shared memory framebuffer transport for zero-copy frame composition.

- **Remote Automation & Control**:
  - Built-in HTTP automation server (default port `9222`) supporting automated navigation, screenshots, DOM dumps, input injection, and JS evaluation.

---

## Getting Started

### Prerequisites

- **Rust toolchain**: 1.85 or newer (`cargo`, `rustc`).
- **Operating System**: Linux (X11 or Wayland).
- **Graphics Drivers**: Vulkan or OpenGL drivers with compute shader support.

### Building from Source

Build the release binary using the project's build script:

```bash
./scripts/cargo.sh build --release
```

### Installation

To install the browser binary to your local user environment (`~/.local/bin/rix-browser`):

```bash
./scripts/install.sh
```

To specify a custom prefix (e.g. `/usr/local`):

```bash
PREFIX=/usr/local ./scripts/install.sh
```

---

## Usage

### Interactive Windowed Mode

Launch the browser in native windowed mode:

```bash
./scripts/cargo.sh run --release -p rix-app -- --windowed --url https://example.org/
```

Specify initial dimensions and custom theme:

```bash
./scripts/cargo.sh run --release -p rix-app -- --windowed --width 1440 --height 900 --theme dark --url https://example.org/
```

Use vertical tab strip:

```bash
./scripts/cargo.sh run --release -p rix-app -- --windowed --vertical-tabs
```

### Headless & CI Automation Mode

Run without a native display window, exposing the HTTP automation API:

```bash
./scripts/cargo.sh run --release -p rix-app -- --headless --control-port 9222 --url https://example.org/
```

### Command-Line Options

| Option | Description |
|:---|:---|
| `--windowed`, `--no-headless` | Open with a native Wayland / X11 window. |
| `--headless` | Run in background without a native window (ideal for CI / automation). |
| `--url <URL>` | Address or search query to open at startup. |
| `--control-port <PORT>` | Automation HTTP API port (default: `9222`). |
| `--width <PX>` | Initial window width in pixels. |
| `--height <PX>` | Initial window height in pixels. |
| `--theme <dark\|light\|custom>` | Chrome shell theme mode. |
| `--horizontal-tabs` | Horizontal tab strip positioned at the top (default). |
| `--vertical-tabs` | Vertical tab strip positioned on the left side rail. |
| `--version`, `-v` | Display version information and exit. |
| `--help`, `-h` | Display available command-line flags. |

---

## Automation API Reference

When launched, RixBrowser serves an HTTP automation endpoint on port `9222` (or the port specified by `--control-port`):

| Method | Endpoint | Description | Payload Example |
|:---|:---|:---|:---|
| `GET` | `/status`, `/health` | Check browser health, current URL, and tab info | *None* |
| `POST` | `/navigate` | Navigate active tab to a new URL | `{"url": "https://example.org"}` |
| `POST` | `/screenshot` | Capture current framebuffer to PNG | `{"path": "/tmp/screenshot.png"}` |
| `GET` | `/dom` | Retrieve serialized DOM tree of active page | *None* |
| `POST` | `/eval` | Evaluate JavaScript in the active page context | `{"script": "document.title"}` |
| `POST` | `/click` | Dispatch mouse click at coordinates (x, y) | `{"x": 150.0, "y": 200.0}` |
| `POST` | `/type` | Dispatch text string input | `{"text": "Search query"}` |
| `POST` | `/key` | Dispatch single keypress event | `{"key": "Enter", "code": "Enter"}` |

### Example cURL Commands

```bash
# Check status
curl -s http://localhost:9222/status

# Navigate to a URL
curl -s -X POST http://localhost:9222/navigate -H "Content-Type: application/json" -d '{"url": "https://example.org"}'

# Take a screenshot
curl -s -X POST http://localhost:9222/screenshot -H "Content-Type: application/json" -d '{"path": "/tmp/screen.png"}'

# Evaluate JavaScript
curl -s -X POST http://localhost:9222/eval -H "Content-Type: application/json" -d '{"script": "1 + 1"}'
```

---

## Testing

Run the full workspace unit and integration test suite:

```bash
./scripts/cargo.sh test --workspace --offline
```

Run tests for an individual crate:

```bash
./scripts/cargo.sh test -p rix-net --offline
```

---

## License

This project is licensed under the **MIT License**. See the [LICENSE](LICENSE) file for details.
