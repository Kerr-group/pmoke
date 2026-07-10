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
pmoke --config config.toml config upgrade # preview a config upgrade
pmoke --config config.toml monitor    # terminal dashboard
pmoke --config config.toml fetch      # fetch waveforms
pmoke --config config.toml analyze    # analyze existing data
pmoke --config config.toml process    # fetch + analyze
pmoke --config config.toml auto       # single + trigger + fetch + analyze
```

If no command is provided, `pmoke` opens `monitor`.

### Upgrade Legacy Configs

Preview the migration to the latest supported config version:

```sh
pmoke --config config.toml config upgrade
```

The preview does not modify files. Write to a new file with `--output`, or use
`--in-place` to create a versioned backup and atomically replace the source:

```sh
pmoke --config config.toml config upgrade --output config.v4.toml
pmoke --config config.toml config upgrade --in-place
```

Potential behavior changes, such as removing a legacy `[timebase]` or changing
the artifact base directory, require `--accept-lossy`. Existing output and
backup files are never overwritten.

## ⚙️ Example Config

```toml
version = 4

[scope]
model = "DHO5108"
connection = "tcp://192.168.10.100:55255"

[generator]
model = "WF1946B"
connection = "gpib://0/11"

[data]
output = "raw"       # "csv", "raw", or "both"
input = "raw"        # "csv", "raw", or "auto"
screenshot = true

[channels]
reference = 2
signals = [3]

[[sensors]]
channel = 1
scale = { max_abs = 55.0, polarity = -1 }
label = "$\\mu_0H$"
unit = "T"

[[sensors]]
channel = 4
scale = { factor = 1.0 }
label = "sensor"
unit = "a.u."

[pulse]
background_before = { start = -5e-3, end = -0.1e-3 }
background_after  = { start = 43e-3, end = 46e-3 }

[reference]
fft_window = { start = 0e-3, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[kerr]
sensor = 1
method = "harmonics" # "standard" or "harmonics"
factor = -1.0

[plot]
mode = "save" # "off", "save", "interactive", or "both"
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

With v4, relative data and plot paths are resolved from the config directory.

Use `data.output = "raw"` for large DHO captures. It preserves the original
WORD payload and avoids huge CSV files in the hot path.

Use `data.input = "raw"` for strict raw analysis, or `"auto"` while
migrating existing data directories.

## 🎛️ Lock-In Notes

The README config uses:

```toml
[lockin]
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }
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

- `channels.reference` is one channel; `channels.signals` is an array.
- Each sensor defines exactly one scale: `{ factor = ... }` or `{ max_abs = ..., polarity = -1|1 }`.
- `max_abs` scales the background-subtracted sensor integral to the requested maximum absolute value.
- `kerr.sensor` must refer to a channel in `sensors`.
- `phase.offsets` must contain six values.
- Time values come from raw metadata or the CSV `time (s)` column.
- Unknown keys in v4 configs are rejected. Legacy v1–v3 configs remain readable.

## 🔌 Hardware Notes

Default builds include hardware support.

- Use `tcp://host:port` for the DHO5108.
- Windows: `visa:RESOURCE` is also supported with NI-VISA installed.
- Install NI-488.2 or `linux-gpib` when using a GPIB function generator.

Screenshot capture uses `:DISPlay:DATA? PNG` and writes directly to:

```text
screenshot/oscilloscope.png
```

GPIB screenshot capture is not supported.

## 🎯 Precision Notes

For DHO5000 large-memory measurements:

- Prefer `data.output = "raw"` or `"both"`.
- Prefer `data.input = "raw"` or `"auto"`.
- Keep `raw_waveform/metadata.toml` with the `chN.u16le` files.
- Generate CSV only when needed for inspection or external tools.

Voltage reconstruction uses:

```text
voltage = (word - y_origin - y_reference) * y_increment
```
