# Vello Renderer Example

> **Legacy prototype.** This example predates the unified `GosubEngine` entry point and drives
> the HTML/CSS/layout pipeline directly. It is kept for reference while the new rendering path
> matures. For new integrations start with [`examples/egui-vello`](../egui-vello) instead.

This example demonstrates how to use a winit window to render a website.

## Usage

```bash
cargo run --release --package vello-renderer -- <URL>
```