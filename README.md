# pmoke — Pulsed MOKE Measurement CLI

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

## 📦 Installation

```sh
cd pmoke
cargo install --path .
