# Jay-A — Study of EasyEffects and Carla

[Português](REFERENCIAS.md) · **English**

> Translation of [REFERENCIAS.md](REFERENCIAS.md). The Portuguese file is the reference and is updated first, so this one may lag behind it.

Notes from reading the code of two open-source audio projects, done at the user's request before writing Jay-A's filters. Both are GPL and Jay-A is **MIT** (decision D6): ideas, structure and parameter ranges come from here, **never code**. The paths quoted are from their repositories.

Read on 2026-11-06: EasyEffects `011bdc5` (2026-11-05), Carla `97a9e07` (2026-03-19).

## 1. EasyEffects (GPL-3.0)

An effects app for PipeWire, on Linux: input (microphone) and output (speakers). The effects themselves come from libraries and plugins (LSP, Calf, RNNoise, DeepFilterNet, SpeexDSP, SoundTouch, libebur128, WebRTC).

### 1.1 The chain

- **An ordered list of instances** (`plugins` in `easyeffects_db_streaminputs.kcfg`; `plugins_order` in the preset), with names like `gate#0` and `compressor#1`. The user picks the order and can repeat an effect. It starts **empty**.
- **Each effect is a node** with a stereo input and output, plus an optional "probe" input for sidechain (`src/plugin_base.hpp`). The echo canceller uses that input as its reference: it's what plays on the speakers.
- **Controls every effect has:** `bypass`, input gain and output gain (−36 to +36 dB), and peak meters at the input and output of each effect (`get_peaks`, `apply_gain` in `src/plugin_base.cpp`).
- **Latency:** each effect reports its own (`get_latency_seconds`), and the chain's total is shown on screen (`src/effects_base.cpp`).
- **Meter and spectrum** sit at the end of the chain, as separate nodes (`output_level`, `spectrum`).

### 1.2 Presets

JSON with one section per chain (`src/presets_manager.cpp`, `src/*_preset.cpp`):

```json
{ "input": { "blocklist": [], "plugins_order": ["gate#0", "compressor#0"],
             "gate#0": { "bypass": false, "input-gain": 0, "output-gain": 0, "attack": 20, "release": 100 } } }
```

It also has community presets, automatic preset loading per device, its own folder of impulses (convolver) and RNNoise models.

### 1.3 Parameter ranges (from `src/contents/kcfg/*.kcfg`)

Ranges and defaults that work as a starting point for voice in Jay-A:

| Effect | Parameters (default [min..max]) |
|---|---|
| Gate | attack 20 ms [0..2000], release 100 ms [0..5000], threshold −24 dB [−60..0] with a −6 dB zone (knee), optional **hysteresis** (a closing threshold separate from the opening one), reduction −24 dB, makeup; Peak/RMS detection; filters on the sidechain (HPF/LPF) |
| Compressor | threshold −12 dB [−60..0], ratio 4 [1..100], knee −6 dB [−24..0], attack 20 ms, release 100 ms, makeup [−60..60]; downward/upward/boost modes; dry/wet |
| Expander | same as the compressor, upward mode by default |
| Limiter | threshold 0 dB [−48..0], **lookahead 5 ms** [0.1..20], attack 5 ms, release 5 ms, optional oversampling and "true peak", automatic leveling (ALR) |
| De-esser | threshold −18 dB, ratio 3 [1..20], detection band at 6 kHz / 4.5 kHz, makeup [0..36] |
| Filter | types: low-pass, high-pass, shelf, bell, band-pass, notch, resonance, all-pass; frequency [10..24000], slope x1..x16, gain [−36..36]; IIR by default (FIR/FFT optional) |
| Equalizer | up to **32 bands** [1..32], each with type, frequency, gain [−36..36], Q and slope; IIR by default |
| RNNoise | wet [−100..20] dB, optional **VAD** with a 50% threshold and a 20 ms **release** (holds the voice open after the VAD drops), swappable models |
| DeepFilterNet | **attenuation limit** 100 dB [0..100], ERB/DF processing thresholds, post-filter; also runs the DPDFNet model |
| Speex | denoise −70 dB, optional AGC, VAD and dereverb |
| Autogain | target −23 LUFS [−100..0], momentary/short/integrated reference (EBU R128, libebur128), 15 s history, silence threshold −70 dB |
| Echo canceller | **WebRTC AudioProcessing**: AEC (with a mobile mode), noise suppression Low..VeryHigh, high-pass, AGC. 10 ms blocks (`src/echo_canceller.cpp`) |
| Pitch | semitones [−12..12], cents, octaves; SoundTouch (LGPL) |

### 1.4 Details that matter

- The **echo canceller is the same engine Jay-A already uses** (WebRTC APM, 10 ms, `ProcessReverseStream` + `ProcessStream`), with the reference through the probe input. It confirms the design in section 5 of [`ARQUITETURA.en.md`](ARQUITETURA.en.md).
- **RNNoise and DeepFilterNet work at 48 kHz**; EasyEffects resamples when the graph runs at another rate. Since Jay-A is all 48 kHz, it doesn't need to.
- **RNNoise's VAD with "release"** is the same idea as the *hold* of Jay-C's gate (`mic-shape.js`): keep a few blocks open after the voice drops, so the end of words isn't cut.
- **The gate's hysteresis** (opening at one level and closing at another, lower one) keeps the gate from opening and closing over and over when the voice is near the threshold. Jay-C's gate doesn't have it.
- **DeepFilterNet comes in as a LADSPA plugin** that the DeepFilterNet project itself publishes (`libDF`, in Rust). Jay-A can use the Rust library directly.

## 2. Carla (GPL-2.0 or later)

A plugin host (LADSPA, DSSI, LV2, VST2, VST3, CLAP, AU, JSFX, SF2/SFZ), with native drivers (JACK, ALSA, CoreAudio, DirectSound/WASAPI through RtAudio).

### 2.1 Processing modes (`source/backend/CarlaBackend.h`, `EngineProcessMode`)

- **Rack** (`CONTINUOUS_RACK`): plugins run **in series, in order**, always in stereo. The output of one is the input of the next.
- **Patchbay:** a free graph, where any output can be connected to any input.
- **Single/Multiple client** (JACK) and **Bridge** (the process that runs one plugin in isolation).

For voice, the rack is enough: it's the same linear chain as EasyEffects. The patchbay is more power than Jay-A needs.

### 2.2 The rack's real-time loop (`source/backend/engine/CarlaEngineGraph.cpp`, `RackGraph::process`)

- For each plugin, in order: if it's off **or can't take the lock right away (`tryLock`)**, it's skipped in this block. **The audio thread never waits** for whoever is touching the plugin (the UI, loading state).
- Between plugins, the buffers are copied (the output becomes the input).

### 2.3 Parameters (`source/backend/CarlaBackend.h`, `ParameterData`, `ParameterRanges`)

- Each parameter has a **default, minimum, maximum, step, small step and large step**, plus a `fixValue` that corrects (clamps) any value it receives.
- It also has type hints: **boolean, integer, logarithmic**, read-only, automatable and not saved.
- **Host controls applied to any plugin** (`PLUGIN_CAN_DRYWET`, `_VOLUME`, `_BALANCE`): dry/wet, volume and balance live outside the plugin, in shared post-processing (`CarlaPluginBridge.cpp`, "Post-processing").

### 2.4 Plugins in another process (`source/backend/plugin/CarlaPluginBridge.cpp`)

- The plugin runs in a **separate process**. Audio goes back and forth through **shared memory**; real-time commands go through a ring of opcodes with a semaphore.
- The host writes "process N frames" and **waits with a timeout** (`waitForClient`). If the plugin doesn't answer, the block is marked as lost, the plugin stops being processed and the UI says so: a hung plugin doesn't bring the host down.
- The protocol has a **version** (`CARLA_PLUGIN_BRIDGE_API_VERSION_MINIMUM` 6, current 10), checked on connection.

It's the same problem as the Jay Link (another process, real time, hangs), solved with shared memory instead of a pipe. Jay-C's hot standby, with its 60 ms without a block, plays the part of Carla's timeout.

## 3. What Jay-A adopts

| Idea | From | Where it goes in Jay-A |
|---|---|---|
| Chain = an ordered list of instances (`gate#0`), chosen by the user, starting from a preset | EasyEffects | The stage 2 rack |
| Every effect with on/off (bypass), input and output gain, input and output meters, reported latency | EasyEffects + Carla (host controls) | The engine's `Effect`: the host applies these the same way for all |
| Parameters with default, min., max., step and scale (linear, log, dB), clamped on input | Carla | The engine's `ParamSpec` and the UI sliders |
| The audio thread never waits for a lock: changes arrive through a lock-free queue, and the new chain goes in whole between two frames | Carla (`tryLock`), adapted | The engine's real-time rule |
| JSON preset, with the order and the parameters of each instance | EasyEffects | `presets/*.json`; import and export |
| Ranges and defaults for voice (section 1.3) | EasyEffects | The stage 2 effects' defaults |
| Gate with hysteresis and hold | EasyEffects + Jay-C | The stage 2 gate |
| Limiter with a 5 ms lookahead | EasyEffects | The limiter at the end of the chain |
| RNNoise with VAD and release; DeepFilterNet with an attenuation limit | EasyEffects | The strong suppression of stage 7 (`nnnoiseless` BSD, `libDF` MIT/Apache) |
| Autogain by loudness (LUFS) | EasyEffects | Normalize in the editor and in conversion (`ebur128` MIT) |
| A separate process with a timeout and a protocol version | Carla | It's already the Jay Link; third-party plugins (D7) would follow the same model |

**Doesn't adopt:** the patchbay (free graph), LV2/VST plugins in v1 (D7), oversampling and dithering in the limiter, the filters' FIR/FFT modes (the latency isn't worth it for live voice), and SoundTouch (LGPL; the candidate is `signalsmith-stretch`, MIT).
