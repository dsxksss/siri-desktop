# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Windows desktop voice assistant (Tauri v2). A transparent floating "Dynamic Island" orb listens for a Chinese wake word, transcribes the following command fully offline (sherpa-onnx), maps it to an intent, and runs a skill (volume, brightness, open app, media keys, NetEase music). Commands the rule engine can't parse fall back to an OpenAI-compatible LLM (DeepSeek by default), which either returns a structured command or answers as chat. Replies are shown on the orb and optionally spoken via offline TTS.

The README (in Chinese) is the authoritative user-facing doc; this file covers what's needed to develop.

## Commands

```bash
npm install                 # frontend deps
npm run tauri dev           # run the app (spawns vite on :1420, then cargo)
npm run build               # tsc + vite build (frontend only)
npm run tauri build         # full release bundle
```

Models are required before the app will start (~250MB into `src-tauri/models/`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/fetch-models.ps1   # kws/ asr/ vad/ tts/
powershell -ExecutionPolicy Bypass -File scripts/package.ps1        # portable build -> dist-portable/
```

There is no test suite. Rust checks: run `cargo check` / `cargo clippy` from `src-tauri/`. The first `cargo build` downloads sherpa-onnx prebuilt **shared** libraries (DLLs) and needs network access.

## Architecture

Two halves talk over Tauri's IPC + an event channel:

- **Rust backend** (`src-tauri/src/`) owns the microphone, all ML inference, and the OS integrations.
- **Frontend** (`src/`, plain TS + Vite, no framework) is just the orb's visual state machine. Two windows: `index.html`/`main.ts` (the orb) and `settings.html`/`settings.ts` (config editor), both declared in `vite.config.ts` rollup inputs and `src-tauri/tauri.conf.json`.

### The voice pipeline (the core)

`pipeline.rs` runs a dedicated `voice-pipeline` thread (the cpal stream is `!Send`, so capture + inference must stay on one thread). It's a two-mode state machine over fixed-size audio chunks:

- **Wake mode** → `wake.rs` (sherpa-onnx KWS) watches for the wake word.
- **Listen mode** → `asr.rs` runs Silero VAD to segment speech, then SenseVoice ASR to transcribe. On a final transcript it calls the `on_text` callback; on timeout it returns to Wake mode.

`audio.rs` captures from cpal and resamples to 16 kHz (`TARGET_RATE`). The thread is controlled via a `crossbeam-channel` of `Control` messages (`Listen` = skip wake word, e.g. orb clicked; `Shutdown` = release mic for restart).

`lib.rs` wires everything in the Tauri `setup` closure and owns the `on_text` callback, which is the **central dispatch**: `intent::parse` (rules) → if `Unknown`, `intent::llm::classify` (LLM) → `skills::dispatch` → `emit_state` to the orb (+ optional TTS). `VoiceManager` (in `lib.rs`) holds the running `PipelineHandle` and rebuilds it on config/mic change (`restart()` stops the thread, reloads config, respawns).

### Intent → skills

- `intent/rules.rs` (`parse`) regex-matches Chinese commands into the `Intent` enum (`intent/mod.rs`).
- `intent/llm.rs` (`classify`) is the fallback: returns `LlmOutcome::Command(Intent)` or `LlmOutcome::Chat(String)`.
- `skills/dispatch` matches an `Intent` to a skill and returns a `Reply { ok, message }`. Skills: `volume.rs` (Win32 Core Audio), `brightness.rs` (`brightness` crate), `open_app.rs` (config `[apps]` map → path/command), `media.rs` (media keys), `netease.rs` (song name → song id → `orpheus://song/{id}` URI; direct API or local NeteaseCloudMusicApi service).

**To add a command:** add a variant to `Intent` (`intent/mod.rs`), a rule in `rules.rs` and/or the LLM prompt in `llm.rs`, a skill module under `skills/`, and an arm in `skills::dispatch`.

### Frontend contract

The backend pushes `StatePayload { state, text }` on the `assistant://state` event (`events.rs` → `emit_state`). `state` is one of `idle | listening | thinking | acting | error`; `main.ts` listens and drives the orb's CSS (`island.dataset.state`). The orb invokes `manual_listen` on click (vs. drag, distinguished by mouse-move distance) and `get_config` to display the wake word. Backend commands exposed via `invoke_handler`: `manual_listen`, `list_microphones`, `get_config`, `save_config`. `main.ts` also has a non-Tauri branch that loops through states for plain-browser preview.

## Config & secrets

`config.rs::load()` deep-merges (overlay wins per leaf): **`config.toml`** (base, committed) ← **`config.local.toml`** (gitignored, for secrets/local paths). Both are searched across `config_base_dirs()` (cwd, cwd/.., exe dir, exe/..) so the same code works in `tauri dev` (cwd = `src-tauri/`) and installed. Every `Config` field has a default, so the app runs with no config file.

- LLM `api_key` is **not** put in config files — it's read from the environment / `.env` (`SIRI_LLM_API_KEY`, `DEEPSEEK_API_KEY`, or `LLM_API_KEY`). Empty key disables the LLM fallback.
- The real wake word is the token list in `src-tauri/models/kws/keywords.txt`, **not** `config.toml`'s `wake_word` (display-only). See the README for generating new wake-word tokens with `sherpa-onnx-cli text2token`.
- `save_config` and the tray mic picker write to `config.local.toml` and call `VoiceManager::restart`.

## Gotchas

- sherpa-onnx is pinned to `features = ["shared"]` — the static MT libs reference MSVC STL symbols the toolchain may lack. The build script copies `sherpa-onnx-c-api.dll` + `onnxruntime.dll` next to the exe; don't switch to static.
- Autostart is registered **release builds only** (`#[cfg(not(debug_assertions))]`) so dev runs don't register the debug exe in the user's startup.
- ASR/TTS inference threads are clamped to `1..4` via `config::worker_threads()`.
- Windows-only: volume/media APIs are under `#[cfg(windows)]`; brightness needs DDC/CI on external monitors.
