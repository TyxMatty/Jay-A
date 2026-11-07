# Jay-A

**English** · [Português](README.pt-BR.md)

The audio app of the Jay line: microphone filters (an option in [Jay-C](https://github.com/TyxMatty/Jay-C), in place of its own filter), audio editing, a soundboard and format conversion. Windows first.

It's for people who call and stream with Jay-C and want a clean voice (no keyboard, clicks or background voices) and simple audio tools in one light app, without a separate noise-removal subscription.

What's done and what's next: [`ROADMAP.md`](ROADMAP.md). Architecture, flows and stages: [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md) (in Portuguese). Done so far: the window (in Jay-C's Calm, Neon and Soft styles) and the tray icon; in Filters, the microphone with WebRTC's cleanup, a level meter and "Hear my microphone", the effect chain (AI noise removal with DeepFilterNet or RNNoise, filter, gate, equalizer, compressor, de-esser, limiter) and presets; and the Jay Link, which hands the filtered microphone to Jay-C during calls, with echo cancellation. The soundboard, the editor and the converter are still a shell.

## Running

It needs the same tools as Jay-C (Node 22+, Rust with MSVC, and Python with Meson and Ninja for WebRTC's audio processing; see Jay-C's `docs/CONTINUAR.en.md`).

```bash
npm install
npm run dev
```

`npm run dev` opens the Jay-A window (Tauri) with live reload. Closing the window keeps Jay-A in the tray; to quit, right-click its icon there. `jay-a.exe --background` starts it in the tray only (that's how Jay-C starts it). To open just the interface in a browser, without Tauri: `npm run web` (port 5175).

The Rust build takes several GB. To keep it out of OneDrive, create a `.cargo/config.toml` (not tracked by git) pointing elsewhere:

```toml
[build]
target-dir = "C:/Users/<you>/.cargo-target/jay-a"
```

## Tests

```bash
npm test
cd crates/jay-audio && cargo test
```
