# pmoke

`pmoke` is a command-line tool for pulsed Magneto-Optical Kerr Effect
(MOKE) measurements. It controls the oscilloscope and function generator,
fetches waveform data, runs reference/sensor/lock-in/phase/Kerr analysis, and
keeps the measurement settings in a reproducible TOML config.

The current code is optimized for Rigol DHO5000-series oscilloscope workflows,
including large-memory WORD waveform transfer and raw binary preservation.

## Features

- Single TOML config for instruments, channel roles, timing, filtering, plotting,
  and Kerr conversion.
- Hardware commands for oscilloscope single mode, function-generator trigger,
  data fetch, and automated measurement.
- CSV output for compatibility and raw WORD output for large DHO5000 captures.
- Analysis can read either `raw.csv` or preserved raw binary waveform files.
- Reference fitting, sensor background/integral processing, numerical lock-in,
  phase rotation, and Kerr-angle extraction.
- Non-interactive PNG plot generation by default, with optional interactive
  matplotlib windows.
- Live terminal dashboard through `pmoke monitor`.

## Requirements

### Core

- Rust stable
- Python 3
- Python packages:
  - `numpy`
  - `scipy`
  - `matplotlib`
  - `lmfit`
  - `gsplot`

### Hardware I/O

Default builds include hardware support.

- Windows:
  - NI-VISA for TCPIP/USB-TMC VISA resources
  - NI-488.2 when using GPIB
  - Visual C++ Build Tools for Rust native dependencies
- Linux:
  - `linux-gpib` when using GPIB
  - VISA/USB-TMC support as required by the selected backend

For analysis-only use, install without hardware features:

```sh
cargo install --path . --no-default-features
```

## Installation

```sh
cargo install --path .
```

From the repository root during development:

```sh
cargo run -- --config config.toml show
cargo run --no-default-features -- --config config.toml analyze
```

## Commands

```text
pmoke [OPTIONS] [COMMAND]

Options:
  -c, --config <FILE>  Path to the configuration file (default: config.toml)

Commands:
  show         Validate and print the normalized config
  monitor      Open the live terminal dashboard
  single       Set the oscilloscope to single mode
  trigger      Send trigger signal from the function generator
  autoshot     Run single + trigger
  fetch        Fetch oscilloscope data
  automeasure  Run single + trigger + fetch
  reference    Analyze the reference signal
  sensor       Analyze the sensor signal
  li           Run numerical lock-in analysis
  phase        Rotate lock-in phase
  kerr         Calculate Kerr angle
  analyze      Run reference + sensor + lock-in + phase + Kerr
  process      Run fetch + analyze after manual pulse triggering
  auto         Run automeasure + analyze
  completions  Generate shell completion script
```

If no command is provided, `pmoke` opens `monitor`.

Hardware commands are available only in default builds. Analysis commands are
available with `--no-default-features`.

## Typical Workflows

Validate a config:

```sh
pmoke --config config.toml show
```

Fetch data only:

```sh
pmoke --config config.toml fetch
```

Run the full analysis after data already exists:

```sh
pmoke --config config.toml analyze
```

Fetch and analyze:

```sh
pmoke --config config.toml process
```

Fully automated shot:

```sh
pmoke --config config.toml auto
```

## Example `config.toml`

```toml
version = 3

[instruments.function_generator]
connection = { protocol = "gpib", board = 0, address = 11 }
model = "WF1946B"

[instruments.oscilloscope]
connection = { protocol = "tcpip", ip = "192.168.10.100", port = 55255 }
model = "DHO5108"
memory_depth = 200_000_000

[fetch]
output = "csv_and_raw"      # "csv", "raw", or "csv_and_raw"
analysis_input = "auto"    # "csv", "raw", or "auto"

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
label = "test"
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
stride_samples = 1_000
lpf_kind = "sync_iir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_cutoff_ref_ratio = 2e-2
lpf_stopband_atten_db = 60.0
lpf_sync_average_cycles = 1.0
lpf_iir_order = 2
lpf_debug_output = false
lpf_debug_overwrite = false

[phase]
m_omega_t0_offset = ["0", "0", "0", "0", "0", "0"]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"    # "standard" or "harmonics"
factor = -1.0
```

## Instrument Connections

Supported connection protocols:

```toml
connection = { protocol = "gpib", board = 0, address = 11 }
connection = { protocol = "tcpip", ip = "192.168.10.100", port = 55255 }
connection = { protocol = "usbtmc", resource = "USB0::0x1AB1::0x0450::DHO5A27090041::INSTR" }
```

USB-TMC uses a VISA resource string. On Windows, confirm the exact resource name
with NI MAX or a VISA resource listing tool before putting it in the config.

## Fetch Output

`pmoke fetch` uses `[fetch].output` unless overridden on the command line:

```toml
[fetch]
output = "csv"              # writes raw.csv
output = "raw"              # writes raw_waveform/
output = "csv_and_raw"      # writes both
```

Command-line overrides:

```sh
pmoke fetch --format csv --out raw.csv
pmoke fetch --format raw --out raw_waveform
pmoke fetch --format csv-and-raw
```

`--out` is accepted for `csv` and `raw`. It is intentionally rejected for
`csv-and-raw`; that mode writes the standard `raw.csv` and `raw_waveform/`
outputs together.

### CSV Output

CSV output writes converted voltage data to:

```text
raw.csv
```

Current CSV output includes a `time (s)` column followed by channel voltage
columns. This is convenient for inspection and compatibility, but very large
captures can become slow and large. For 200 Mpts data, prefer raw WORD output as
the primary archive format.

### Raw WORD Output

Raw output writes:

```text
raw_waveform/
  metadata.toml
  ch1.u16le
  ch2.u16le
  ch3.u16le
  ...
```

Each `chN.u16le` file stores the little-endian DHO WORD payload exactly as
received. The metadata stores the raw preamble for audit and separately queried
scaling values used for reconstruction:

- `x_increment`, `x_origin`, `x_reference`
- `y_increment`, `y_origin`, `y_reference`
- `vertical_offset`, `vertical_scale`
- sample count and channel file names

The `x_*` and `y_*` values are queried with `WAV:XINC?`, `WAV:XOR?`,
`WAV:XREF?`, `WAV:YINC?`, `WAV:YOR?`, and `WAV:YREF?`, not parsed from rounded
`WAV:PRE?` fields.

Voltage reconstruction uses the DHO scaling formula:

```text
voltage = (word - y_origin - y_reference) * y_increment
```

Use raw output when precision and reproducibility matter. It preserves the
oscilloscope WORD data and avoids CSV formatting as the primary storage format.

## Analysis Input

Analysis commands read waveform input according to `[fetch].analysis_input`:

```toml
[fetch]
analysis_input = "csv"   # read raw.csv with a time column
analysis_input = "raw"   # read raw_waveform/metadata.toml and chN.u16le
analysis_input = "auto"  # prefer complete raw_waveform/, otherwise raw.csv
```

`raw` is strict: missing metadata or channel files is an error.
`csv` requires a time column in current `version = 3` configs. Legacy `version =
2` configs can still use their deprecated `[timebase]` as a fallback for old
CSV files without a time column.

`auto` is useful while migrating. It uses raw binary data only when
`raw_waveform/metadata.toml` and all required channel files are complete. If
`raw_waveform/` is absent, it falls back to `raw.csv`.

## Generated Files

Typical analysis output:

```text
raw.csv
raw_waveform/
lockin_results_ch3.csv
lockin_rotated_ch3.csv
kerr_results.csv
plots/
  reference_fit.png
  sensor_raw.png
  sensor_integral.png
  lockin_results.png
  phase_rotated.png
  omega_t0_analysis.png
  kerr_ch3.png
```

For multiple signal channels, lock-in, rotated, and Kerr files are generated per
configured signal channel.

## Plot Settings

Plotting is configured under `[plot]`:

```toml
[plot]
enabled = true
save = true
interactive = false
output_dir = "plots"
max_points = 100_000
decimation = "stride"
fail_on_error = false
```

Defaults are chosen for unattended analysis:

- `interactive = false`: do not open GUI windows during analysis.
- `save = true`: write PNG files.
- `output_dir = "plots"`: keep generated images out of the top-level shot
  directory.
- `max_points = 100_000`: decimate plot data only. Analysis still uses the full
  data.
- `fail_on_error = false`: plot failures are reported as warnings unless this is
  set to `true`.

Set `interactive = true` only when you want each plot window to block progress
until the window is closed.

## Config Notes

`version = 3` is the normalized schema used by the current code.

Important rules:

- `roles.reference_ch` is a single channel index.
- `roles.sensor_ch` and `roles.signal_ch` are arrays.
- Sensor channels used for field/current conversion must define `factor`,
  `label`, and `unit_out`.
- `kerr.use_sensor_ch` must be one of `roles.sensor_ch`.
- `phase.m_omega_t0_offset` must contain six values, one for each harmonic.
- `[timebase]` is no longer used. Time axis values come from raw waveform
  metadata or the CSV `time (s)` column.
- Unknown keys in `version = 3` are rejected.
- Unused `[[channels]]` entries are allowed but produce warnings.

`version = 1` and `version = 2` configs are still read and normalized where
migration is unambiguous. `version = 2` `[timebase]` is treated as deprecated
legacy fallback data for old CSV files. Use `pmoke show` to inspect the
normalized result.

## Lock-in LPF

`lockin.lpf_kind` selects the low-pass filter after complex demodulation:

- `sync_iir_zero_phase`: recommended starting point for MHz modulation and
  millisecond-scale pulse data. It applies short synchronous averaging followed
  by forward/backward Butterworth IIR filtering for zero-phase offline analysis.
- `fir_zero_phase`: complex-baseband FIR filtering with a symmetric
  Kaiser-windowed low-pass filter.
- `fir_boxcar_enbw`: FIR comparison mode that approximately matches the
  equivalent noise bandwidth of the legacy boxcar weighting.
- `boxcar_legacy`: old moving-average/trapezoidal-integration style lock-in,
  useful for continuity with older datasets.

For `fir_zero_phase` and `sync_iir_zero_phase`, set exactly one cutoff:

```toml
lpf_cutoff_hz = 20_000.0
# or
lpf_cutoff_ref_ratio = 2e-2
```

The cutoff must fit within the output sampling rate:

```text
output_rate = 1 / (x_increment * lockin.stride_samples)
cutoff_hz < 0.45 * output_rate
```

Example starting point for `f_ref = 1 MHz`:

```toml
[lockin]
lpf_kind = "sync_iir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_cutoff_ref_ratio = 2e-2
lpf_sync_average_cycles = 1.0
lpf_iir_order = 2
```

If the result is noisy, try `lpf_iir_order = 4` or a smaller cutoff. If the pulse
shape is too rounded, use a larger cutoff. `lpf_half_window_cycles` describes
support width for FIR-related calculations; it is not a universal precision
setting.

## Lock-in Debug Output

Enable debug output when comparing filters:

```toml
[lockin]
lpf_debug_output = true
lpf_debug_label = "trial_001"
lpf_debug_overwrite = false
snr_background_window = { start = -5e-3, end = -0.1e-3 }
snr_signal_window = { start = 0e-3, end = 5e-3 }
```

Files are written under:

```text
lockin_debug/{label}/{lpf_kind}_ch{ch}_h{m}/
```

Typical files:

- `metadata.csv`
- `filter_response.csv`
- `baseband_psd.csv`
- `snr_summary.csv`

Debug windows affect only diagnostics. They do not change the lock-in result.

## Precision Guidance

For DHO5000 large-memory measurements:

- Prefer `fetch.output = "raw"` or `"csv_and_raw"` for acquisition.
- Prefer `fetch.analysis_input = "raw"` or `"auto"` for analysis.
- Keep raw WORD files and `metadata.toml` as the primary archived data.
- Use CSV for compatibility, quick inspection, or exported subsets.
- Avoid BYTE waveform transfer for precision-sensitive work.

The raw path preserves the original WORD bytes and stores the exact parsed
preamble values used for voltage reconstruction.
