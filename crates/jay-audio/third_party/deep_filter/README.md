# deep_filter (DeepFilterNet's libDF), for Jay-A

A copy of `libDF` from DeepFilterNet's tag `v0.5.6` (commit `978576aa8400552a4ce9730838c635aa30db5e61`,
https://github.com/Rikorose/DeepFilterNet), with its built-in model `models/DeepFilterNet3_onnx.tar.gz`.
Code and model weights are MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

**Why a copy:** v0.5.6 needs tract 0.19, and every tract 0.19 has RUSTSEC-2026-0217 (an
out-of-bounds read in tract-nnef's NNEF tensor reader). There's no newer DeepFilterNet release,
its main branch needs tract 0.21, whose fixed versions (0.21.16+) pin `time <0.3.42` while Tauri
needs `time ^0.3.47`, and main also changes the processing (it skips frames after a quiet stretch).

**What changed from v0.5.6, nothing else:**
- `libDF/Cargo.toml`: tract-core, tract-onnx, tract-pulse and tract-hir `^0.19.4` → `^0.22.2`;
  ndarray `^0.15` → `^0.16` (the one tract 0.22 uses).
- `libDF/src/tract.rs`: `m.symbol_table.sym("S")` → `m.symbols.sym("S")` (3 lines; the field was
  renamed in tract).

Measured: on 12 s of speech in 10 ms frames, the output is bit for bit the same as v0.5.6 on
tract 0.19. Jay-A only uses `DfParams::default()` (the built-in model) and `DfTract`.

**Drop the copy** when DeepFilterNet releases a version on a fixed tract that builds with Tauri.
