<div align="center">

# 💥 pmoke

**High-Performance Pulsed MOKE Measurement & Waveform Demodulation System**

[![CI](https://github.com/Kerr-group/pmoke/actions/workflows/ci.yml/badge.svg)](https://github.com/Kerr-group/pmoke/actions/workflows/ci.yml)
[![Docs Site](https://img.shields.io/badge/docs-online-6366f1?style=flat-square&logo=github-pages&logoColor=white)](https://kerr-group.github.io/pmoke/)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square)](LICENSE)

[📖 Documentation Site](https://kerr-group.github.io/pmoke/) | [⚡ Quickstart](#-quickstart) | [🎛️ CLI Reference](#%EF%B8%8F-cli-reference) | [✨ Features](#-key-features)

---

</div>

`pmoke` is a modern, high-precision command-line tool for **pulsed Magneto-Optical Kerr Effect (MOKE)** measurements and automated waveform analysis. Built in Rust and WebAssembly, it seamlessly orchestrates hardware instruments, processes multi-channel oscilloscopes, and executes complete lock-in demodulation chains.

```text
reference ➔ sensor integral ➔ lock-in demodulation ➔ phase rotation ➔ Kerr angle
```

---

## ✨ Key Features

- ⚡ **High-Speed Binary Waveform Ingestion**: Direct support for Rigol DHO5000-series 16-bit WORD binary captures (`.u16le`), avoiding CSV bottlenecks in large memory acquisitions.
- 🎛️ **Full Lock-In Demodulation**: Hardware/software lock-in with zero-phase FIR/IIR filtering and boxcar demodulation.
- 📊 **Interactive Terminal Dashboard**: Real-time activity telemetry with `pmoke monitor` featuring paused-history navigation and calm status metrics.
- 🌐 **WebAssembly & Web Interactive Tools**: Interactive waveform analysis and live diagnostic tools running directly in the browser via Wasm.
- 🔧 **Multi-Transport Hardware Control**: Native SCPI controller over TCP/Ethernet, Direct GPIB, and Prologix USB/Serial bridges.
- 🛡️ **Immutable Shot Provenance**: TOML-based reproducible configuration (`version = 4`) with isolated atomic run directories.

---

## ⚡ Quickstart

### 1. Installation

Build and install `pmoke` using Cargo (Rust 1.85+ required):

```bash
# Hardware-enabled build (Direct TCP / Prologix / GPIB)
cargo install --path .

# Analysis-only build (Lightweight CLI for offline processing)
cargo install --path . --no-default-features
```

Install Python plotting and analysis dependencies:

```bash
pip install -r requirements.txt
```

### 2. Basic Workflow

```bash
# 1. Validate configuration
pmoke --config config.toml show

# 2. Check hardware connections & instrument diagnostics
pmoke --config config.toml doctor

# 3. Automated single-shot pulse, fetch & analysis
pmoke --config config.toml auto

# 4. Launch live terminal monitor dashboard
pmoke --config config.toml monitor
```

---

## 🎛️ CLI Reference

| Command | Description |
| :--- | :--- |
| `pmoke show` | Validate configuration syntax and hardware bindings |
| `pmoke doctor` | Preflight check for instruments, Python environment, and storage |
| `pmoke auto` | Trigger single shot, capture waveforms, and run analysis |
| `pmoke monitor` | Open the interactive TUI activity dashboard |
| `pmoke fetch` | Retrieve raw waveforms from oscilloscope |
| `pmoke analyze` | Re-run lock-in & Kerr angle analysis on existing captures |
| `pmoke export csv` | Convert verified raw binary `.u16le` waveforms to CSV |
| `pmoke instruments list` | List supported instrument drivers and protocols |

---

## 📁 Repository Structure

```text
pmoke/
├── crates/
│   ├── pmoke-core/      # Core numerical analysis & Rust engine
│   └── pmoke-web-wasm/  # WebAssembly bindings for docs site & browser tools
├── website/             # Fumadocs static documentation site & interactive tools
├── scripts/             # Python analysis & benchmark utilities
└── config.toml          # Example configuration file
```

---

## 📖 Complete Documentation

Visit our official documentation website for comprehensive guides, CLI reference, configuration schema, and interactive waveform tools:

👉 **[https://kerr-group.github.io/pmoke/](https://kerr-group.github.io/pmoke/)**

- 🚀 [Quickstart Guide](https://kerr-group.github.io/pmoke/docs/quickstart)
- 💻 [CLI Reference](https://kerr-group.github.io/pmoke/docs/cli/reference)
- ⚙️ [Configuration Reference](https://kerr-group.github.io/pmoke/docs/configuration/reference)
- 📊 [Interactive Waveform Analyzer](https://kerr-group.github.io/pmoke/docs/interactive/waveform-analyzer)

---

## 📚 Publications & References

If you use `pmoke` in your scientific research, please consider citing the following publications:

- **Magneto-optical Kerr-effect measurements under pulsed magnetic fields over 40 T using a compact sample fixture**  
  Atsutoshi Ikeda, Sota Nakamura, Soichiro Yamane, Kosuke Noda, Akihiko Ikeda, and Shingo Yonezawa  
  *Phys. Rev. Research* (2026). DOI: [10.1103/vy7j-ylb4](https://journals.aps.org/prresearch/abstract/10.1103/vy7j-ylb4)

- **Magneto-optical Kerr effect measurements under bipolar pulsed magnetic fields**  
  Soichiro Yamane, Sota Nakamura, Atsutoshi Ikeda, Kosuke Noda, Akihiko Ikeda, and Shingo Yonezawa  
  *JJAP Conf. Proc.* **12**, 011011 (2026). Article: [J-STAGE 12_011011](https://www.jstage.jst.go.jp/article/jjapcp/12/0/12_011011/_article/-char/ja)

---

## 📄 License

This project is licensed under the [Apache 2.0 License](LICENSE).
