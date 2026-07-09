# 💥 pmoke

`pmoke` is a command-line tool for pulsed MOKE measurements.

It controls the oscilloscope and function generator, fetches waveform data, and
runs the full analysis chain:

```text
reference -> sensor integral -> lock-in -> phase rotation -> Kerr angle
```

The current workflow is tuned for Rigol DHO5000-series oscilloscopes, large
WORD waveform captures, and reproducible TOML-based analysis.

## ✨ What It Does

- Runs hardware shots, waveform fetch, screenshots, and analysis from one config.
- Stores large captures as raw DHO WORD files with scaling metadata.
- Reads either `raw_waveform/` or `raw.csv` for analysis.
- Produces lock-in, phase-rotated, Kerr, and PNG plot outputs.
- Opens a live terminal dashboard with `pmoke monitor`.

## ⚡ Install

Hardware-enabled build:

```sh
cargo install --path .
```

Analysis-only build:

```sh
cargo install --path . --no-default-features
```

Development commands:

```sh
cargo run -- --config config.toml show
cargo run --release -- --config config.toml process
cargo run --release --no-default-features -- --config config.toml analyze
```

## 🚀 Commands

```text
pmoke --config config.toml show       # validate config
pmoke --config config.toml monitor    # terminal dashboard
pmoke --config config.toml fetch      # fetch waveforms
pmoke --config config.toml analyze    # analyze existing data
pmoke --config config.toml process    # fetch + analyze
pmoke --config config.toml auto       # single + trigger + fetch + analyze
```

If no command is provided, `pmoke` opens `monitor`.

## ⚙️ Example Config

```toml
version = 3

[instruments.function_generator]
connection = { protocol = "gpib", board = 0, address = 11 }
model = "WF1946B"

[instruments.oscilloscope]
connection = { protocol = "tcpip", ip = "192.168.10.100", port = 55255 }
model = "DHO5108"

[fetch]
output = "raw"          # "csv", "raw", or "csv_and_raw"
analysis_input = "raw"  # "csv", "raw", or "auto"

[screenshot]
enabled = true

[plot]
enabled = true
save = true
interactive = false
output_dir = "plots"
max_points = 100_000
decimation = "stride"
fail_on_error = false

[roles]
sensor_ch = [1, 4]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = -39364.84663082185
label = "$\\mu_0H$"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[[channels]]
index = 4
factor = 1.0
label = "sensor"
unit_out = "a.u."

[pulse]
bg_window_before = { start = -5e-3, end = -0.1e-3 }
bg_window_after  = { start = 43e-3, end = 46e-3 }

[reference]
fft_window = { start = 0e-3, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
workers = 2
stride_samples = 100
lpf_kind = "boxcar_legacy"
lpf_half_window_cycles = 1.0
lpf_debug_output = false
lpf_debug_overwrite = false

[phase]
m_omega_t0_offset = ["0", "0", "0", "0", "0", "0"]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics" # "standard" or "harmonics"
factor = -1.0
```

## 📁 Data Layout

Typical files after acquisition and analysis:

```text
raw_waveform/
  metadata.toml
  ch1.u16le
  ch2.u16le
  ...
screenshot/
  oscilloscope.png
lockin_results_ch3.csv
lockin_rotated_ch3.csv
kerr_results.csv
plots/
```

Use `fetch.output = "raw"` for large DHO captures. It preserves the original
WORD payload and avoids huge CSV files in the hot path.

Use `fetch.analysis_input = "raw"` for strict raw analysis, or `"auto"` while
migrating existing data directories.

## 🎛️ Lock-In Notes

The README config uses:

```toml
lpf_kind = "boxcar_legacy"
stride_samples = 100
```

`boxcar_legacy` keeps continuity with the older moving-average style lock-in.
`stride_samples = 100` keeps dense output, which is useful if you want to apply
additional smoothing later.

Other LPF modes are available:

- `sync_iir_zero_phase`
- `fir_zero_phase`
- `fir_boxcar_enbw`
- `boxcar_legacy`

For FIR/IIR cutoff-based modes, keep the output rate high enough:

```text
output_rate = 1 / (x_increment * lockin.stride_samples)
cutoff_hz < 0.45 * output_rate
```

## ✅ Config Rules

- `roles.reference_ch` is one channel.
- `roles.sensor_ch` and `roles.signal_ch` are arrays.
- Sensor channels must define `factor`, `label`, and `unit_out`.
- `kerr.use_sensor_ch` must be included in `roles.sensor_ch`.
- `phase.m_omega_t0_offset` must contain six values.
- Time values come from raw metadata or the CSV `time (s)` column.
- Unknown keys in `version = 3` configs are rejected.

## 🔌 Hardware Notes

Default builds include hardware support.

- Windows: install NI-VISA, and NI-488.2 when using GPIB.
- Linux: install `linux-gpib` when using GPIB.
- USB-TMC uses a VISA resource string.

Screenshot capture uses `:DISPlay:DATA? PNG` and writes directly to:

```text
screenshot/oscilloscope.png
```

GPIB screenshot capture is not supported.

## 🎯 Precision Notes

For DHO5000 large-memory measurements:

- Prefer `fetch.output = "raw"` or `"csv_and_raw"`.
- Prefer `fetch.analysis_input = "raw"` or `"auto"`.
- Keep `raw_waveform/metadata.toml` with the `chN.u16le` files.
- Generate CSV only when needed for inspection or external tools.

Voltage reconstruction uses:

```text
voltage = (word - y_origin - y_reference) * y_increment
```
