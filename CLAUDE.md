# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Schrottbar is a Wayland status bar written in Rust. It renders on the wlr-layer-shell protocol using software rendering (tiny-skia) with cosmic-text for font shaping. It uses tokio for async runtime and supports multiple monitors.

## Build & Run

```bash
cargo build          # debug build
cargo run            # run (requires a Wayland compositor with wlr-layer-shell)
cargo clippy         # lint
cargo test           # tests (currently none defined)
```

Logs go to `/var/log/schrottbar/` (file) and stderr (terminal). The log directory must exist and be writable.

Config files are read from `~/.config/schrottbar/`.

## Architecture

**Main loop** (`src/main.rs`): Tokio async select loop handling three event sources — Wayland events, signals, and inter-component messages via channels.

**Channel system** (`src/state_item.rs`): Two channel types coordinate the bar:
- `MainAction` (mpsc): items → main loop. Sends `Redraw`, `Reinit`, `Terminate`.
- `ItemAction` (broadcast): main loop → items. Sends `Update`, `Terminate`.

**StateItem trait** (`src/state_item.rs`): Each bar item implements `StateItem` with:
- `print()` — writes content into a `SectionWriter` for a given output (monitor name)
- `start_coroutine()` — spawns a tokio task that periodically updates state and requests redraws

**SectionWriter** (`src/section_writer.rs`): Builds a list of `ContentItem`s with powerline-style separators. Items call `open(bg, fg)` / `open_bg(bg)` → `write(text)` → `close()` to emit styled segments. Handles automatic powerline arrow insertion between segments.

**Rendering pipeline**: `Bar` (`src/bar.rs`) → `Renderer` (`src/renderer.rs`). Bar creates wl_shm buffers, renderer draws text/shapes into tiny-skia pixmaps. Buffer byte order is BGRA (wl_shm Argb8888 on little-endian), so R↔B channels are swapped when converting RGBA colors to tiny-skia paints.

**Wayland integration** (`src/wayland.rs`): Uses smithay-client-toolkit. `WaylandState` manages outputs, layer surfaces, and shm pools. One layer surface per monitor, anchored top with exclusive zone.

**Items** (`src/items/`): Each item is a self-contained module with its own coroutine:
- `time` — clock display (30s refresh)
- `system` — CPU/RAM usage via sysinfo (k10temp Tctl for CPU temp)
- `weather` — weather via wttr.in
- `pulseaudio` — volume display
- `paymo` — time tracker integration
- `test_display` — development/debugging item

**Types** (`src/types.rs`): Core types — `RGBA` tuple, `ContentItem`, `ContentShape` (Text or Powerline), alignment and powerline enums. These are re-exported via `src/lib.rs` for the `examples/` crate.

## Key Details

- Font is hardcoded to "UbuntuMono Nerd Font Propo" at 29px in `bar.rs`
- Items are hardcoded in `init_state_items()` in `main.rs` (no config-driven item list yet)
- Edition 2024 Rust
- The `compositor/` module contains niri-specific window title tracking (not yet wired into items)
