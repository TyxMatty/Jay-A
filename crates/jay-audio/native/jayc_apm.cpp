// Minimal C API over WebRTC's AudioProcessing (echo cancellation, noise suppression, gain),
// so Rust can call it without bindgen. Frames are 10 ms of interleaved float samples in [-1, 1].
#include <memory>
#include <vector>

#include "modules/audio_processing/include/audio_processing.h"

namespace {

struct Apm {
  rtc::scoped_refptr<webrtc::AudioProcessing> apm;
  webrtc::StreamConfig capture;
  webrtc::StreamConfig render;
  std::vector<std::vector<float>> capture_planes;
  std::vector<std::vector<float>> render_planes;
};

void Deinterleave(const float* in, size_t frames, std::vector<std::vector<float>>& planes) {
  const size_t channels = planes.size();
  for (size_t c = 0; c < channels; ++c) {
    planes[c].resize(frames);
    for (size_t i = 0; i < frames; ++i) planes[c][i] = in[i * channels + c];
  }
}

void Interleave(const std::vector<std::vector<float>>& planes, size_t frames, float* out) {
  const size_t channels = planes.size();
  for (size_t c = 0; c < channels; ++c)
    for (size_t i = 0; i < frames; ++i) out[i * channels + c] = planes[c][i];
}

std::vector<float*> Pointers(std::vector<std::vector<float>>& planes) {
  std::vector<float*> pointers;
  for (auto& plane : planes) pointers.push_back(plane.data());
  return pointers;
}

}  // namespace

extern "C" {

// Returns nullptr on failure. `echo_cancellation` = 0 leaves AEC off (e.g. headphone users);
// `noise_suppression` = 0 leaves NS off (e.g. playing an instrument); `auto_gain` = 0 leaves AGC off.
void* jayc_apm_create(int sample_rate, int capture_channels, int render_channels, int echo_cancellation,
                      int noise_suppression, int auto_gain) {
  auto state = std::make_unique<Apm>();
  state->apm = webrtc::AudioProcessingBuilder().Create();
  if (!state->apm) return nullptr;

  webrtc::AudioProcessing::Config config;
  config.pipeline.maximum_internal_processing_rate = 48000;
  config.high_pass_filter.enabled = true;
  config.echo_canceller.enabled = echo_cancellation != 0;
  config.echo_canceller.mobile_mode = false;
  config.noise_suppression.enabled = noise_suppression != 0;
  config.noise_suppression.level = webrtc::AudioProcessing::Config::NoiseSuppression::kHigh;
  config.gain_controller2.enabled = auto_gain != 0;
  config.gain_controller2.adaptive_digital.enabled = auto_gain != 0;
  state->apm->ApplyConfig(config);

  state->capture = webrtc::StreamConfig(sample_rate, capture_channels);
  state->render = webrtc::StreamConfig(sample_rate, render_channels);
  state->capture_planes.resize(capture_channels);
  state->render_planes.resize(render_channels);
  return state.release();
}

// Feeds 10 ms of what the speakers are playing (the echo reference). Returns 0 on success.
int jayc_apm_process_render(void* handle, const float* interleaved, int frames) {
  auto* state = static_cast<Apm*>(handle);
  if (frames != static_cast<int>(state->render.num_frames())) return -1;
  Deinterleave(interleaved, frames, state->render_planes);
  auto pointers = Pointers(state->render_planes);
  return state->apm->ProcessReverseStream(pointers.data(), state->render, state->render, pointers.data());
}

// Cleans 10 ms of microphone audio in place. Returns 0 on success.
int jayc_apm_process_capture(void* handle, float* interleaved, int frames, int delay_ms) {
  auto* state = static_cast<Apm*>(handle);
  if (frames != static_cast<int>(state->capture.num_frames())) return -1;
  Deinterleave(interleaved, frames, state->capture_planes);
  auto pointers = Pointers(state->capture_planes);
  state->apm->set_stream_delay_ms(delay_ms);
  const int result = state->apm->ProcessStream(pointers.data(), state->capture, state->capture, pointers.data());
  Interleave(state->capture_planes, frames, interleaved);
  return result;
}

void jayc_apm_destroy(void* handle) {
  delete static_cast<Apm*>(handle);
}

}  // extern "C"
