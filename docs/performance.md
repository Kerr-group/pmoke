# Performance

pmoke tracks elapsed time and peak memory separately. Shared CI runners are
useful for trends, but their timing is not a release gate.

Run the deterministic synthetic workload locally:

```console
cargo bench --locked --no-default-features --bench performance
```

Select a workload size and save the machine-readable report:

```console
cargo bench --locked --no-default-features --bench performance -- \
  --samples 10000000 --iterations 5 --output results.json
```

For peak resident memory on Linux, wrap the command with `/usr/bin/time -v`.
On macOS, use `/usr/bin/time -l`. The weekly `Performance` workflow stores the
JSON report, resource usage, commit, toolchain, CPU, OS, and architecture as an
artifact. The 50-million-sample workload is reserved for a manual or
self-hosted run.

The benchmark contains deterministic RAW WORD decoding, sensor integration,
and `boxcar_legacy` lock-in workloads. Expected numerical behavior remains in
the regular golden and unit tests; benchmark timing alone never defines
correctness.
