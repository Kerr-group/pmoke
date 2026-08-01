use anyhow::{Context, Result, bail};
use instruments::transport::{ScpiConnection, open_scpi_transport};
use std::env;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Args {
    host: String,
    port: u16,
    pad: u8,
    timeout_ms: u16,
    iterations: usize,
    warmup: usize,
    command: String,
    show_responses: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            host: "10.249.11.17".to_string(),
            port: 1234,
            pad: 17,
            timeout_ms: 3_000,
            iterations: 50,
            warmup: 3,
            command: "*IDN?".to_string(),
            show_responses: false,
        }
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.iterations == 0 {
        bail!("--iterations must be greater than 0");
    }

    let connection = ScpiConnection::PrologixTcp {
        host: args.host.clone(),
        port: args.port,
        address: args.pad,
        read_timeout_ms: args.timeout_ms,
    };

    eprintln!("Connecting to {connection}");
    eprintln!(
        "Benchmarking {:?}: warmup={}, iterations={}",
        args.command, args.warmup, args.iterations
    );

    let mut transport = open_scpi_transport(&connection).context("failed to open Prologix TCP")?;

    for _ in 0..args.warmup {
        let _ = transport
            .query_line(&args.command)
            .with_context(|| format!("warmup query failed: {}", args.command))?;
    }

    let mut samples = Vec::with_capacity(args.iterations);
    let mut last_response = String::new();
    for index in 0..args.iterations {
        let start = Instant::now();
        let response = transport
            .query_line(&args.command)
            .with_context(|| format!("query {} failed: {}", index + 1, args.command))?;
        let elapsed = start.elapsed();

        if args.show_responses {
            println!(
                "{:>4}: {:>10.3} ms  {}",
                index + 1,
                ms(elapsed),
                response.trim()
            );
        }

        last_response = response;
        samples.push(elapsed);
    }

    samples.sort_unstable();
    let total = samples
        .iter()
        .copied()
        .fold(Duration::ZERO, |acc, value| acc + value);
    let mean = total.as_secs_f64() * 1_000.0 / samples.len() as f64;
    let throughput = 1_000.0 / mean;

    println!();
    println!("Keithley 2010 Prologix TCP benchmark");
    println!("  endpoint   : {}:{}", args.host, args.port);
    println!("  gpib pad   : {}", args.pad);
    println!("  command    : {}", args.command);
    println!("  iterations : {}", args.iterations);
    println!("  last reply : {}", last_response.trim());
    println!();
    println!("Latency");
    println!("  min   {:>10.3} ms", ms(samples[0]));
    println!("  p50   {:>10.3} ms", ms(percentile(&samples, 50.0)));
    println!("  p90   {:>10.3} ms", ms(percentile(&samples, 90.0)));
    println!("  p99   {:>10.3} ms", ms(percentile(&samples, 99.0)));
    println!("  max   {:>10.3} ms", ms(samples[samples.len() - 1]));
    println!("  mean  {:>10.3} ms", mean);
    println!("  rate  {:>10.2} query/s", throughput);

    Ok(())
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut values = env::args().skip(1);

    while let Some(flag) = values.next() {
        match flag.as_str() {
            "--host" => args.host = next_value(&mut values, &flag)?,
            "--port" => args.port = next_value(&mut values, &flag)?.parse()?,
            "--pad" => args.pad = next_value(&mut values, &flag)?.parse()?,
            "--timeout-ms" => args.timeout_ms = next_value(&mut values, &flag)?.parse()?,
            "--iterations" | "-n" => args.iterations = next_value(&mut values, &flag)?.parse()?,
            "--warmup" => args.warmup = next_value(&mut values, &flag)?.parse()?,
            "--command" | "-c" => args.command = next_value(&mut values, &flag)?,
            "--show-responses" => args.show_responses = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => bail!("unknown argument: {flag}; use --help"),
        }
    }

    Ok(args)
}

fn next_value(values: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    values
        .next()
        .with_context(|| format!("missing value for {flag}"))
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn percentile(samples: &[Duration], percentile: f64) -> Duration {
    debug_assert!(!samples.is_empty());
    let rank = (percentile / 100.0) * (samples.len().saturating_sub(1) as f64);
    samples[rank.round() as usize]
}

fn print_help() {
    println!(
        "\
Benchmark Keithley 2010 SCPI query latency through Prologix Ethernet.

Usage:
  cargo run -p instruments --example keithley2010_prologix_bench --no-default-features --features prologix-tcp -- [options]

Options:
  --host HOST            Prologix host (default: 10.249.11.17)
  --port PORT            Prologix TCP port (default: 1234)
  --pad PAD              GPIB primary address (default: 17)
  --timeout-ms MS        Prologix read timeout, 1..=3000 (default: 3000)
  -n, --iterations N     Measured query count (default: 50)
  --warmup N             Warmup query count (default: 3)
  -c, --command COMMAND  SCPI query command (default: *IDN?)
  --show-responses       Print every measured response
  -h, --help             Print this help
"
    );
}
