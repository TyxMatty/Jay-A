# Jay-A roadmap

Jay-A is the audio app of the Jay line: microphone filters for [Jay-C](https://github.com/TyxMatty/Jay-C) calls, a soundboard, an audio editor and format conversion. Windows first.

Every milestone ends with something testable. The full plan, with the "done when" checks and the measurements of each finished milestone, is in [`docs/ARQUITETURA.md`](docs/ARQUITETURA.md) (in Portuguese), section 12.

## Done

- [x] **0. Scaffold:** Tauri v2 + Vite + vanilla JS, Jay-C's three styles (Calm, Neon, Soft), tray icon, single instance and `--background`.
- [x] **1. Audio engine:** WASAPI microphone capture with WebRTC AudioProcessing (echo cancellation, noise suppression, automatic gain), "Hear my microphone" monitor and level meter.
- [x] **2a. Effect chain engine:** parameters with ranges and scales, a reorderable rack swapped without locks, 10 ms crossfades, JSON presets and zero allocations on the audio thread.
- [x] **2b. Effects:** filter, gate (with hysteresis and hold), 10-band equalizer, compressor, de-esser and limiter, plus the app's 4 presets. The whole chain uses about 0.5% of a CPU core.
- [x] **2c. Filters tab:** build the chain by mouse or keyboard, controls generated from each parameter's spec, per-effect meters, total latency and presets (save, import, export).
- [x] **2d. AI noise removal:** DeepFilterNet 3 and RNNoise as effects in the chain, for keyboard, clicks and background voices.

## Next

- [ ] **3. Jay Link server:** send the filtered voice to Jay-C over a local named pipe. Closing Jay-A mid-call must not cut the voice (Jay-C's own filter takes over), and other apps' sound must not be muffled. In progress.
- [ ] **4. Soundboard:** clips played by a global hotkey, with a game in focus, reaching the call and the headphones without coming back as echo.
- [ ] **5. Format conversion:** batch conversion to WAV, FLAC, MP3, OGG and Opus, where a corrupted file fails on its own and the others still convert.
- [ ] **6. Audio editor:** cut, fade, normalize (LUFS) and export, with undo and a preview on the monitor.
- [ ] **7. Voice effects:** pitch, robot and reverb as chain effects, with latency and CPU measured.
- [ ] **8. Soundboard inside Jay-C:** a button in the call bar that lists and plays Jay-A's clips.
- [ ] **9. Release:** an NSIS installer in `%LOCALAPPDATA%\Jay-A`, offered by Jay-C. Both apps installed on a clean machine make a filtered call, and Jay-C alone keeps working with its own filter.

## Open questions

- Soundboard with the microphone muted in Jay-C: mute everything (simple), or a separate soundboard track on the server.
- An English version of the architecture document.
- Third-party plugins (VST3/CLAP) in the chain, each in its own process: after milestone 7.
