# Jay-A roadmap

[Português](ROADMAP.md) · **English**

Translation of [`ROADMAP.md`](ROADMAP.md); the Portuguese version is the reference and may be ahead of this one.

Jay-A is the audio app of the Jay line: microphone filters for [Jay-C](https://github.com/TyxMatty/Jay-C) calls and other apps, a soundboard, an audio editor and format conversion. Windows first.

Every stage ends with something testable. The full plan, with the "done when" checks and the measurements of each finished stage, is in [`docs/ARQUITETURA.en.md`](docs/ARQUITETURA.en.md), section 12.

## Done

- [x] **0. Scaffold:** Tauri v2 + Vite + vanilla JS, Jay-C's three styles (Calm, Neon, Soft), tray icon, single instance and `--background`.
- [x] **1. Audio engine:** WASAPI microphone capture with WebRTC AudioProcessing (echo cancellation, noise suppression, automatic gain), "Hear my microphone" monitor and level meter.
- [x] **2a. Effect chain engine:** parameters with ranges and scales, a reorderable rack swapped without locks, 10 ms crossfades, JSON presets and zero allocations on the audio thread.
- [x] **2b. Effects:** filter, gate (with hysteresis and hold), 10-band equalizer, compressor, de-esser and limiter, plus the app's 4 presets. The whole chain uses about 0.5% of a CPU core.
- [x] **2c. Filters tab:** build the chain by mouse or keyboard, controls generated from each parameter's spec, per-effect meters, total latency and presets (save, import, export).
- [x] **2d. AI noise removal:** DeepFilterNet 3 and RNNoise as effects in the chain, for keyboard, clicks and background voices.
- [x] **3. Jay Link server:** the filtered microphone goes to Jay-C over a local named pipe that only the same user can open, every 20 ms (p99 20.4 ms in the installed app), with Jay-C's call audio as the echo reference. The audio threads run with Windows' "Pro Audio" priority, so a busy CPU doesn't stall them.
- [x] **4. Soundboard:** clips imported from any common format, played by global hotkeys with another app in focus, mixed into the call and the headphones, with "stop all" and the clips removed from the echo.
- [x] **Microphone for other apps:** the filtered microphone and the soundboard reach Discord, OBS and others through the free VB-CABLE, which Jay-A downloads, checks and installs from VB-Audio's own setup.
- [x] **5. Format conversion:** batch conversion to WAV, FLAC, MP3, OGG and Opus, one task per core, where a corrupted file fails on its own and the others still convert.
- [x] **6. Audio editor:** cut, fade, gain, normalize, reverse and trim silence, with undo, a preview in the headphones and export to any conversion format.
- [x] **Installer and updates:** a per-user NSIS installer (no admin rights) on GitHub Releases, inside the Jay-I setup; installed and updated from the [Jay-H](https://github.com/TyxMatty/Jay-H) hub and by the app itself (a pill announces the new version).
- [x] **Welcome tour and English docs.**

## Next

- [ ] **Jay Link in a real call** (with Jay-C's I5): the echo removed, closing Jay-A mid-call doesn't cut the voice (Jay-C's own filter takes over), and other apps' sound isn't muffled.
- [ ] **Editor, the rest:** speed, loudness normalization (LUFS) and applying a filter preset to a file.
- [ ] **7. Voice effects:** pitch, robot and reverb as chain effects, with latency and CPU measured.
- [ ] **8. Soundboard inside Jay-C:** a button in the call bar that lists and plays Jay-A's clips.
- [ ] **9. Release with Jay-C:** Jay-C offers Jay-A; both installed on a clean machine make a filtered call, and Jay-C alone keeps working with its own filter.

## Open questions

- Soundboard with the microphone muted in Jay-C: mute everything (simple), or a separate soundboard track on the server.
- Third-party plugins (VST3/CLAP) in the chain, each in its own process: after stage 7.
