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
version = 1

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
reference_ch = [2] # Supports only one reference channel
signal_ch = [3,4]

# Channel for current sensor

[[channels]]
index = 1
factor = -39364.84663082185
label = "$B$"
unit_out = "T"

# Channel for signal

[[channels]]
index = 2

[[channels]]
index = 3

[[channels]]
index = 4
factor = 1
label = "$V$"
unit_out = "V"

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
demodulation = "complex"
lpf_kind = "fir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_stopband_atten_db = 60.0
# deprecated:
# filter_length_samples = 1

[phase]
use_signal_ch = [3,4]
m_omega_t0_offset = ["0", "0", "0", "0", "0", "0"]

[kerr]
use_sensor_ch = 1            # Supports only one sensor channel
kerr_type     = "harmonics"  # "standard" or "harmonics"
factor        = 1
```

---

## 📘 Notes

`config.toml` defines all instrument connections, channel roles, timing settings, lock-in parameters, and Kerr-analysis settings.

A minimal configuration is enough to run `pmoke process` or `pmoke auto`.

`lockin.lpf_kind` selects the low-pass filter applied after complex demodulation.

- `fir_zero_phase`: Default. Applies a symmetric FIR low-pass filter to the complex baseband and exports the result as legacy `LIx/LIy`. This is the recommended mode for better stopband rejection and improved S/N without changing the downstream `omega_t0` fitting flow.
- `boxcar_legacy`: Keeps the previous moving-average / boxcar-style lock-in behavior for comparison with older results.

`lpf_half_window_cycles = 1.0` corresponds to the former `filter_length_samples = 1` setting. It means the half-window is one reference cycle, so the effective lock-in window spans about two reference cycles.

For backward compatibility, configs that only set the deprecated `filter_length_samples` field keep using `boxcar_legacy` unless `lpf_kind` is explicitly set.
