# Keithley examples

Examples for Keithley digital multimeters.

## Keithley 2010 over Prologix Ethernet

Target setup:

- Prologix Ethernet controller: `10.249.11.17:1234`
- GPIB primary address: `17`
- Instrument: Keithley 2010
- Query: `*IDN?`

Run the Rust example:

```bash
cargo run --locked -p instruments --example keithley2010_prologix_tcp --no-default-features --features prologix-tcp
```

Expected output:

```text
Connecting to Prologix TCP 10.249.11.17:1234, address 17
*IDN? -> KEITHLEY INSTRUMENTS INC.,MODEL 2010,...
```

The example uses:

- `++mode 1`
- `++addr 17`
- `++auto 0`
- `++eoi 1`
- `++eos 0`
- `++read_tmo_ms 3000`
- `*IDN?`
- `++read eoi`

`prologix-rs` validates `read_timeout_ms` as `1..=3000`, so use `3000` or less.

## PyVISA socket check

Use this when you want to verify the controller and instrument independently from
the Rust transport layer.

Install dependencies:

```bash
python -m pip install pyvisa pyvisa-py
```

Run:

```bash
python crates/instruments/examples/keithley/keithley2010_prologix_pyvisa.py
```

Override connection settings if needed:

```bash
python crates/instruments/examples/keithley/keithley2010_prologix_pyvisa.py \
  --host 10.249.11.17 \
  --port 1234 \
  --pad 17 \
  --timeout-ms 3000
```

## Troubleshooting

If Rust fails before connecting with `invalid Prologix read timeout`, lower
`READ_TIMEOUT_MS` in the example to `3000` or less.

If PyVISA connects but returns no response, check that the Keithley GPIB address
is `17` from the instrument front panel.

If both Rust and PyVISA fail to connect, check network reachability first:

```bash
nc -vz 10.249.11.17 1234
```

If socket connection works but `*IDN?` times out, try a direct Prologix version
query to confirm the controller responds:

```text
++ver
```

For Prologix commands sent over raw socket, each command must be terminated with
LF (`\n`).

## Query latency benchmark

Use this to measure Prologix TCP + GPIB + Keithley response latency.

```bash
cargo run --locked -p instruments --example keithley2010_prologix_bench --no-default-features --features prologix-tcp
```

Default benchmark settings:

- host: `10.249.11.17`
- port: `1234`
- GPIB PAD: `17`
- command: `*IDN?`
- warmup: `3`
- measured iterations: `50`

Example output:

```text
Keithley 2010 Prologix TCP benchmark
  endpoint   : 10.249.11.17:1234
  gpib pad   : 17
  command    : *IDN?
  iterations : 50
  last reply : KEITHLEY INSTRUMENTS INC.,MODEL 2010,...

Latency
  min        12.345 ms
  p50        13.210 ms
  p90        15.001 ms
  p99        20.442 ms
  max        20.442 ms
  mean       13.721 ms
  rate       72.88 query/s
```

Change the query and count:

```bash
cargo run --locked -p instruments --example keithley2010_prologix_bench --no-default-features --features prologix-tcp -- \
  --iterations 100 \
  --warmup 5 \
  --command "*IDN?"
```

Print every measured response:

```bash
cargo run --locked -p instruments --example keithley2010_prologix_bench --no-default-features --features prologix-tcp -- \
  --show-responses
```

For pure communication overhead, `*IDN?` is a safe first benchmark. Measurement
queries such as `:READ?` include Keithley integration time, trigger state, and
range settings, so they are not directly comparable unless the instrument setup
is fixed first.
