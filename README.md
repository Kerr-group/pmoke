# 💥pmoke — Pulsed MOKE Measurement CLI

`pmoke` is a command-line tool designed to control a pulsed Magneto-Optical Kerr Effect (MOKE) measurement system.  
It automates oscilloscope control, trigger handling, data fetching, numerical lock-in analysis, phase rotation, and Kerr angle extraction, enabling fully reproducible experiments and analysis pipelines.

This tool is intended for research use in laboratories performing MOKE measurements under pulsed magnetic fields.

---

## 🚀 Features

- Configure measurement devices from a single TOML file  
- Control oscilloscope modes (single, fetch, trigger synchronization)  
- Send trigger signals from a function generator  
- Perform automated measurements (single → trigger → fetch)  
- Numerical lock-in analysis  
- Automatic phase rotation based on our zero-area Sagnac interferometer system
- Calculate Kerr angle
- Run a full analysis pipeline with a single command (`process`)  
- Fully automated measurement + analysis workflow (`auto`)  
- Shell completion script generation

---

## ⛓️ Dependencies

- Rust (latest stable version recommended)
- Python
- Python packages:
  - numpy
  - scipy
  - matplotlib
  - lmfit
  - gsplot

### Windows

- NI-VISA runtime (for GPIB/TCPIP/USB instrument communication)
- NI-488.2 driver (for GPIB communication)
- Visual C++ Build Tools (for compiling Rust dependencies)

### Linux

- linux-gpib (for GPIB communication)

---

## 📦 Installation

```sh
cd pmoke
cargo install --path .

# Without instrument drivers
cd pmoke
cargo install --path . --no-default-features
```

---

## 🧭 Usage

```sh
A CLI tool to conduct pulsed MOKE

Usage: pmoke [OPTIONS] [COMMAND]

Commands:
  show         Display the contents of the configuration file
  single       Set single mode to the oscilloscope
  trigger      Send trigger signal from the function generator
  autoshot     Set single mode and send trigger signal
  fetch        Fetch data from the oscilloscope and save to a file
  automeasure  Perform auto measurement (set single mode, trigger, fetch)
  reference    Analyze the reference signal
  sensor       Analyze the sensor signal
  li           Run numerical lock-in analysis
  phase        Rotate the reference phase for lock-in analysis
  kerr         Calculate the Kerr angle
  process      Automated analysis after manually triggering the pulse
               (fetch, lock-in, phase, Kerr)
  auto         Run the full automatic measurement and analysis
  completions  Generate shell completion script
  help         Print this message or the help of the given subcommand(s)

Options:
  -c, --config <FILE>  Path to the configuration file (default: ./config.toml)
  -h, --help           Print help
  -V, --version        Print version
```

---

## ⚙ Example config.toml

Below is an example configuration file used with `pmoke`:

```toml
version = 2

[instruments.function_generator]
connection = { protocol = "gpib", board = 0, address = 11 }
model = "WF1946B"

[instruments.oscilloscope]
connection = { protocol = "tcpip", ip = "10.249.11.19", port = 55255 }
model = "DHO5108"
memory_depth = 10_000_000

[timebase]
t0 = -0.5e-3 # seconds
dt = 500e-12 # seconds

[roles]
sensor_ch = [1]
reference_ch = 3
signal_ch = [2]

# Channel for current sensor

[[channels]]
index = 1
factor = -39364.84663082185
label = "$B$"
unit_out = "T"

# Channel for signal

[[channels]]
index = 2

# Channel for reference

[[channels]]
index = 3

[pulse]
bg_window_before = { start = -5e-3, end = -0.1e-3 }
bg_window_after  = { start = 4.2e-3, end = 15e-3 }

[reference]
fft_window = { start = 0e-3, end = 5e-3 }
stride_samples = 100_000
window_samples = 1_000

[lockin]
workers = 4
stride_samples = 1_000
lpf_kind = "fir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_stopband_atten_db = 60.0

[phase]
m_omega_t0_offset = ["0", "0", "0", "0", "0", "0"]

[kerr]
use_sensor_ch = 1            # Supports only one sensor channel
kerr_type     = "harmonics"  # "standard" or "harmonics"
factor        = 1
```

---

## 📘 Notes

`config.toml` defines all instrument connections, channel roles, timing settings, lock-in parameters, and Kerr-analysis settings.

A `version = 2` config is the normalized schema used internally by the current code. Unknown keys in `version = 2` are rejected, so `pmoke show` is the easiest way to confirm that your file matches the expected structure.

`pmoke show` behaves differently depending on the config state.

- Runnable config: prints warnings, then prints the normalized `version = 2` config.
- Non-runnable config: prints diagnostics and stops before any measurement or analysis command runs.

This is useful when migrating old files, because `version = 1` configs are still readable and normalized on load.

## 🔧 Config schema

The main structural changes in the current schema are:

- `roles.reference_ch` is a single channel index, not an array.
- `phase.use_signal_ch` is removed. Phase rotation and Kerr analysis always target `roles.signal_ch`.
- Unused `[[channels]]` entries are allowed, but they produce a warning instead of an error.
- Most analysis commands still require `instruments.oscilloscope.memory_depth`, because the time axis and lock-in indexing are reconstructed from the oscilloscope sampling settings.

Legacy `version = 1` configs are still accepted. During normalization:

- `roles.reference_ch = [n]` is migrated to `roles.reference_ch = n`
- `lockin.filter_length_samples` is migrated to `lockin.lpf_half_window_cycles`
- if a legacy file only uses `lockin.filter_length_samples`, `lpf_kind` defaults to `boxcar_legacy` for backward-compatible behavior
- deprecated `phase.use_signal_ch` is rejected if it differs from `roles.signal_ch`
- deprecated `lockin.demodulation` is ignored because complex demodulation is now always used

## 🎚️ Lock-in LPF

`lockin.lpf_kind` selects the low-pass filter applied after complex demodulation.

- `fir_zero_phase`: Complex-baseband FIR mode. The signal is demodulated to complex baseband, filtered by a symmetric FIR, and then exported as legacy `LIx/LIy`.
- `boxcar_legacy`: Previous moving-average / trapezoidal-integration style lock-in, kept for comparison with older datasets.

`fir_zero_phase` keeps the downstream phase-fitting flow unchanged. The `omega_t0` rotation is still handled later by the existing fitting step. The lock-in only changes how the complex baseband is formed and low-pass filtered.

The current `fir_zero_phase` path works as follows:

1. The reference analysis determines `f_ref` and `omega_tref`.
2. For harmonic `m`, the raw signal is multiplied by `exp(-i m (omega t - omega_tref))` to shift that harmonic to baseband.
3. A symmetric odd-length FIR is applied to the complex baseband at each output center.
4. Because the taps are symmetric around the output center, the filter is zero-phase with respect to the sampled lock-in points. There is no additional group delay to compensate in the later `omega_t0` fit.
5. The filtered complex result `z` is exported using the legacy convention `LIx = -Im(z)` and `LIy = Re(z)`, so existing downstream phase rotation and Kerr code can stay unchanged.

The FIR taps are currently designed from two quantities:

- `lpf_half_window_cycles`: Sets the half-width of the support window in reference cycles.
- `lpf_stopband_atten_db`: Sets the Kaiser-window attenuation parameter used when shaping the FIR taps.

The support width is determined by:

```text
t_half = lpf_half_window_cycles / f_ref
n_half = floor(t_half / dt)
tap_count = 2 * n_half + 1
```

So `lpf_half_window_cycles = 1.0` means:

- the half-window is one reference cycle
- the full FIR support is about two reference cycles
- the number of taps is determined from that physical width and the oscilloscope sample interval `dt`

This parameter describes the FIR support width only. It should not be interpreted as "the same setting as the old two-cycle lock-in" or as a universally safe default.

The current implementation chooses a low-pass cutoff from the support width and the lock-in output rate:

```text
raw_cutoff_hz = 0.5 / t_half
output_rate = 1 / (dt * stride_samples)
cutoff_hz = min(raw_cutoff_hz, 0.45 * output_rate)
```

This means `stride_samples` does not only thin out the saved lock-in points. It also limits the maximum usable low-pass bandwidth, because the filtered result is only evaluated every `stride_samples` samples.

Compared with `boxcar_legacy`, `fir_zero_phase` is not expected to give numerically identical results even when `lpf_half_window_cycles` is set to the same nominal width. The legacy path uses separate sine/cosine mixing plus finite-window integration, whose frequency response is sinc-like and has relatively large sidelobes. The FIR path instead uses complex baseband filtering with a Kaiser-windowed low-pass response, so leakage, noise folding, and apparent signal amplitude can change noticeably.

With the current implementation, `lpf_half_window_cycles = 1.0` and `fir_zero_phase` should be read as "an FIR supported over about two cycles", not as "legacy `filter_length_samples = 1` reproduced in FIR form". If continuity with old data matters, compare against `boxcar_legacy` first and tune from there.

In practice:

- Use `fir_zero_phase` when you explicitly want complex-baseband FIR filtering and are willing to re-check the resulting amplitude and phase behavior on real data.
- Use `boxcar_legacy` when you need continuity with old results or want a direct A/B comparison during migration.
- Treat `lpf_half_window_cycles = 1.0` as a description of support width, not as a recommended universal starting point.
