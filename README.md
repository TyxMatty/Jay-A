# Jay-A

**English** · [Português](README.pt-BR.md)

The audio app of the Jay line: microphone filters (an option in [Jay-C](https://github.com/TyxMatty/JayC), in place of its own filter), audio editing, a soundboard and format conversion. Windows first.

Architecture, flows and stages: [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md) (in Portuguese). For now the app has its window (in Jay-C's Calm, Neon and Soft styles), the tray icon and the microphone test in Filters (noise suppression, automatic gain, a level meter and "Hear my microphone"). The rest is still a shell.

## Running

It needs the same tools as Jay-C (Node 22+, Rust with MSVC, and Python with Meson and Ninja for WebRTC's audio processing; see Jay-C's `docs/CONTINUAR.en.md`).

```bash
npm install
npm run dev
```

`npm run dev` opens the Jay-A window (Tauri) with live reload. Closing the window keeps Jay-A in the tray; to quit, right-click its icon there. `jay-a.exe --background` starts it in the tray only (that's how Jay-C will start it). To open just the interface in a browser, without Tauri: `npm run web` (port 5175).

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
