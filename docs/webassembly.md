# WebAssembly

The engine can be compiled to WebAssembly via `wasm-pack`:

```bash
wasm-pack build --target web
```

Then serve the thin UA wrapper in `wasm/`:

```bash
cd wasm
bun run dev   # or: npm run dev
```

To run the demo you need a Chromium with WebGPU enabled:

```bash
# Linux only — PRs welcome for Windows / macOS
chromium --disable-web-security --enable-features=Vulkan \
         --enable-unsafe-webgpu --user-data-dir=/tmp/chromium-temp-profile
```

![Browser in browser](../resources/images/browser-wasm-hackernews.png)
