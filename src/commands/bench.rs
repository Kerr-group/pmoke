use crate::cli::{BenchCommand, BenchProtocol};
use crate::commands::instruments::{
    QueryConnection, TextQuerySession, configured_query_timeout_ms, display_query_connection,
    open_text_query_session, parse_query_connection, validate_line_request,
    validate_scpi_query_command,
};
use crate::ui;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const REPORT_SCHEMA_VERSION: u32 = 1;
const SCPI_QUERY_REPORT_SCHEMA_VERSION: u32 = 1;
const SCPI_QUERY_REPORT_DIR: &str = "benchmarks/scpi-query";
const MAX_REQUESTS: usize = 64;
const MAX_ITERATIONS: usize = 100_000;
const MAX_WARMUP: usize = 100_000;
const MAX_TOTAL_STORED_SAMPLES: usize = 200_000;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Serialize)]
struct TransportBenchmarkReport {
    schema_version: u32,
    generated_at: String,
    connection: String,
    protocol: &'static str,
    timeout_ms: u64,
    warmup: usize,
    iterations: usize,
    results: Vec<RequestBenchmark>,
}

#[derive(Debug, Serialize)]
struct RequestBenchmark {
    request: String,
    attempts: usize,
    success_count: usize,
    timeout_count: usize,
    error_count: usize,
    warmup_failure_count: usize,
    success_rate: f64,
    timeout_rate: f64,
    error_rate: f64,
    latency_ms: Option<LatencySummary>,
    last_response: Option<String>,
    samples: Vec<BenchmarkSample>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SampleOutcome {
    Success,
    Timeout,
    Error,
}

#[derive(Debug, Serialize)]
struct BenchmarkSample {
    iteration: usize,
    outcome: SampleOutcome,
    elapsed_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip)]
    failure_kind: Option<FailureKind>,
}

#[derive(Debug, Serialize)]
struct LatencySummary {
    min: f64,
    mean: f64,
    p50: f64,
    p90: f64,
    p99: f64,
    max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FailureKind {
    Timeout,
    Connection,
    UnsupportedFeature,
    Protocol,
    Other,
}

impl FailureKind {
    const ALL: [Self; 5] = [
        Self::Timeout,
        Self::Connection,
        Self::UnsupportedFeature,
        Self::Protocol,
        Self::Other,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connection => "connection",
            Self::UnsupportedFeature => "unsupported_feature",
            Self::Protocol => "protocol",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Serialize)]
struct ScpiQueryBenchmarkReport {
    schema_version: u32,
    pmoke_version: &'static str,
    git_commit: String,
    timestamp: String,
    os: &'static str,
    arch: &'static str,
    connection: String,
    protocol: &'static str,
    command: String,
    timeout_ms: u64,
    iterations: usize,
    warmup: usize,
    warmup_failure_count: usize,
    success_count: usize,
    timeout_count: usize,
    error_count: usize,
    errors_by_kind: BTreeMap<&'static str, usize>,
    min_ms: Option<f64>,
    p50_ms: Option<f64>,
    p90_ms: Option<f64>,
    p99_ms: Option<f64>,
    max_ms: Option<f64>,
    mean_ms: Option<f64>,
    query_per_sec: f64,
    last_response: Option<String>,
}

pub fn run(command: &BenchCommand, run_dir: Option<&Path>, force: bool) -> Result<()> {
    match command {
        BenchCommand::ScpiQuery {
            connection,
            command,
            iterations,
            warmup,
            timeout_ms,
            output,
            json,
        } => run_scpi_query(ScpiQueryOptions {
            connection,
            command,
            iterations: *iterations,
            warmup: *warmup,
            timeout_ms: *timeout_ms,
            output: output.as_deref(),
            run_dir,
            json: *json,
            force,
        }),
        BenchCommand::Transport {
            connection,
            protocol,
            requests,
            iterations,
            warmup,
            timeout_ms,
            output,
            json,
        } => run_transport(TransportOptions {
            connection,
            protocol: *protocol,
            requests,
            iterations: *iterations,
            warmup: *warmup,
            timeout_ms: *timeout_ms,
            output: output.as_deref(),
            json: *json,
            force,
        }),
    }
}

struct ScpiQueryOptions<'a> {
    connection: &'a str,
    command: &'a str,
    iterations: usize,
    warmup: usize,
    timeout_ms: u64,
    output: Option<&'a Path>,
    run_dir: Option<&'a Path>,
    json: bool,
    force: bool,
}

struct TransportOptions<'a> {
    connection: &'a str,
    protocol: BenchProtocol,
    requests: &'a [String],
    iterations: usize,
    warmup: usize,
    timeout_ms: u64,
    output: Option<&'a Path>,
    json: bool,
    force: bool,
}

fn run_scpi_query(options: ScpiQueryOptions<'_>) -> Result<()> {
    validate_scpi_query_options(&options)?;
    validate_scpi_query_command(options.command)?;
    let connection = parse_query_connection(options.connection, options.timeout_ms)?;
    let total = options
        .warmup
        .checked_add(options.iterations)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("benchmark operation count is too large"))?;
    let progress = (!options.json).then(|| ui::progress("SCPI query benchmark", total));
    let mut session = None;
    let result = benchmark_request(
        &connection,
        options.timeout_ms,
        options.command,
        options.warmup,
        options.iterations,
        &mut session,
        progress.as_ref(),
    );
    if let Some(progress) = progress {
        if result.success_count > 0 {
            ui::finish_success(progress, "SCPI query benchmark completed");
        } else {
            ui::finish_warning(
                progress,
                "SCPI query benchmark completed without a response",
            );
        }
    }

    let report = compact_scpi_report(&connection, options.timeout_ms, options.warmup, &result);
    let encoded_toml = toml::to_string_pretty(&report)
        .context("failed to encode SCPI query benchmark report as TOML")?;
    let saved_path = save_scpi_query_report(
        &report.timestamp,
        encoded_toml.as_bytes(),
        options.output,
        options.run_dir,
        options.force,
    )?;

    if options.json {
        let mut encoded_json =
            serde_json::to_vec_pretty(&report).context("failed to encode benchmark JSON")?;
        encoded_json.push(b'\n');
        io::stdout()
            .lock()
            .write_all(&encoded_json)
            .context("failed to write benchmark JSON")?;
    } else {
        print_scpi_query_report(&report);
        ui::saved(format!("SCPI query benchmark: {}", saved_path.display()));
    }

    if result.success_count == 0 {
        bail!("SCPI query benchmark completed without a successful response");
    }
    Ok(())
}

fn validate_scpi_query_options(options: &ScpiQueryOptions<'_>) -> Result<()> {
    if !(1..=MAX_ITERATIONS).contains(&options.iterations) {
        bail!("--iterations must be in 1..={MAX_ITERATIONS}");
    }
    if options.warmup > MAX_WARMUP {
        bail!("--warmup must be in 0..={MAX_WARMUP}");
    }
    if options.timeout_ms == 0 {
        bail!("--timeout-ms must be positive");
    }
    Ok(())
}

fn compact_scpi_report(
    connection: &QueryConnection,
    timeout_ms: u64,
    warmup: usize,
    result: &RequestBenchmark,
) -> ScpiQueryBenchmarkReport {
    let mut errors_by_kind = FailureKind::ALL
        .into_iter()
        .map(|kind| (kind.as_str(), 0))
        .collect::<BTreeMap<_, _>>();
    for kind in result
        .samples
        .iter()
        .filter_map(|sample| sample.failure_kind)
    {
        *errors_by_kind.entry(kind.as_str()).or_default() += 1;
    }
    let measured_seconds = result
        .samples
        .iter()
        .map(|sample| sample.elapsed_ms)
        .sum::<f64>()
        / 1_000.0;
    let query_per_sec = if measured_seconds > 0.0 {
        result.success_count as f64 / measured_seconds
    } else {
        0.0
    };
    let latency = result.latency_ms.as_ref();

    ScpiQueryBenchmarkReport {
        schema_version: SCPI_QUERY_REPORT_SCHEMA_VERSION,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        git_commit: option_env!("PMOKE_GIT_COMMIT")
            .unwrap_or("unknown")
            .to_string(),
        timestamp: jiff::Timestamp::now().to_string(),
        os: env::consts::OS,
        arch: env::consts::ARCH,
        connection: display_query_connection(connection),
        protocol: "scpi",
        command: result.request.clone(),
        timeout_ms: configured_query_timeout_ms(connection, timeout_ms),
        iterations: result.attempts,
        warmup,
        warmup_failure_count: result.warmup_failure_count,
        success_count: result.success_count,
        timeout_count: result.timeout_count,
        error_count: result.error_count,
        errors_by_kind,
        min_ms: latency.map(|stats| stats.min),
        p50_ms: latency.map(|stats| stats.p50),
        p90_ms: latency.map(|stats| stats.p90),
        p99_ms: latency.map(|stats| stats.p99),
        max_ms: latency.map(|stats| stats.max),
        mean_ms: latency.map(|stats| stats.mean),
        query_per_sec,
        last_response: result.last_response.clone(),
    }
}

fn run_transport(options: TransportOptions<'_>) -> Result<()> {
    validate_options(&options)?;
    for request in options.requests {
        validate_request(options.protocol, request)?;
    }

    let connection = parse_query_connection(options.connection, options.timeout_ms)?;
    let mut session = Some(open_text_query_session(&connection, options.timeout_ms)?);
    let total = options
        .requests
        .len()
        .checked_mul(options.warmup.saturating_add(options.iterations))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("benchmark operation count is too large"))?;
    let progress = (!options.json).then(|| ui::progress("transport benchmark", total));

    let mut results = Vec::with_capacity(options.requests.len());
    for request in options.requests {
        let result = benchmark_request(
            &connection,
            options.timeout_ms,
            request,
            options.warmup,
            options.iterations,
            &mut session,
            progress.as_ref(),
        );
        results.push(result);
    }
    let has_success = results.iter().any(|result| result.success_count > 0);
    if let Some(progress) = progress {
        if has_success {
            ui::finish_success(progress, "transport benchmark completed");
        } else {
            ui::finish_warning(progress, "transport benchmark completed without a response");
        }
    }

    let report = TransportBenchmarkReport {
        schema_version: REPORT_SCHEMA_VERSION,
        generated_at: jiff::Timestamp::now().to_string(),
        connection: display_query_connection(&connection),
        protocol: protocol_name(options.protocol),
        timeout_ms: configured_query_timeout_ms(&connection, options.timeout_ms),
        warmup: options.warmup,
        iterations: options.iterations,
        results,
    };
    let mut encoded =
        serde_json::to_vec_pretty(&report).context("failed to encode benchmark report")?;
    encoded.push(b'\n');

    if let Some(output) = options.output {
        write_report(output, &encoded, options.force)?;
    }
    if options.json {
        io::stdout()
            .lock()
            .write_all(&encoded)
            .context("failed to write benchmark JSON")?;
    } else {
        print_human_report(&report);
        if let Some(output) = options.output {
            ui::saved(format!("benchmark report: {}", output.display()));
        }
    }

    if !has_success {
        bail!("transport benchmark completed without a successful response");
    }
    Ok(())
}

fn validate_options(options: &TransportOptions<'_>) -> Result<()> {
    if options.requests.is_empty() {
        bail!("at least one --request is required");
    }
    if options.requests.len() > MAX_REQUESTS {
        bail!("at most {MAX_REQUESTS} requests can be benchmarked at once");
    }
    if !(1..=MAX_ITERATIONS).contains(&options.iterations) {
        bail!("--iterations must be in 1..={MAX_ITERATIONS}");
    }
    let total_stored_samples = options
        .requests
        .len()
        .checked_mul(options.iterations)
        .ok_or_else(|| anyhow::anyhow!("benchmark sample count is too large"))?;
    if total_stored_samples > MAX_TOTAL_STORED_SAMPLES {
        bail!(
            "benchmark would store {total_stored_samples} samples; reduce --request or --iterations to at most {MAX_TOTAL_STORED_SAMPLES} total samples"
        );
    }
    if options.warmup > MAX_WARMUP {
        bail!("--warmup must be in 0..={MAX_WARMUP}");
    }
    if options.timeout_ms == 0 {
        bail!("--timeout-ms must be positive");
    }
    Ok(())
}

fn validate_request(protocol: BenchProtocol, request: &str) -> Result<()> {
    match protocol {
        BenchProtocol::Scpi => validate_scpi_query_command(request),
        BenchProtocol::Line => validate_line_request(request),
    }
}

fn benchmark_request(
    connection: &QueryConnection,
    timeout_ms: u64,
    request: &str,
    warmup: usize,
    iterations: usize,
    session: &mut Option<TextQuerySession>,
    progress: Option<&ui::UiProgress>,
) -> RequestBenchmark {
    let mut warmup_failure_count = 0;
    for _ in 0..warmup {
        let outcome = prepare_session(session, connection, timeout_ms)
            .and_then(|()| query_session(session, request));
        if outcome.is_err() {
            warmup_failure_count += 1;
            session.take();
        }
        if let Some(progress) = progress {
            progress.inc(1);
        }
    }

    let mut samples = Vec::with_capacity(iterations);
    let mut successful_latencies = Vec::with_capacity(iterations);
    let mut last_response = None;
    for iteration in 1..=iterations {
        let (outcome, elapsed) = measured_query(session, connection, timeout_ms, request);
        let elapsed_ms = duration_ms(elapsed);
        match outcome {
            Ok(response) => {
                successful_latencies.push(elapsed_ms);
                last_response = Some(response);
                samples.push(BenchmarkSample {
                    iteration,
                    outcome: SampleOutcome::Success,
                    elapsed_ms,
                    error: None,
                    failure_kind: None,
                });
            }
            Err(error) => {
                let failure_kind = classify_failure(&error);
                let outcome = if failure_kind == FailureKind::Timeout {
                    SampleOutcome::Timeout
                } else {
                    SampleOutcome::Error
                };
                samples.push(BenchmarkSample {
                    iteration,
                    outcome,
                    elapsed_ms,
                    error: Some(format!("{error:#}")),
                    failure_kind: Some(failure_kind),
                });
                session.take();
            }
        }
        if let Some(progress) = progress {
            progress.inc(1);
        }
    }

    summarize_request(
        request,
        warmup_failure_count,
        samples,
        successful_latencies,
        last_response,
    )
}

fn measured_query(
    session: &mut Option<TextQuerySession>,
    connection: &QueryConnection,
    timeout_ms: u64,
    request: &str,
) -> (Result<String>, Duration) {
    if session.is_none() {
        let started = Instant::now();
        if let Err(error) = reconnect_session(session, connection, timeout_ms) {
            return (
                Err(error.context("failed to reconnect before a measured request")),
                started.elapsed(),
            );
        }
    }

    let started = Instant::now();
    let outcome = query_session(session, request);
    (outcome, started.elapsed())
}

fn prepare_session(
    session: &mut Option<TextQuerySession>,
    connection: &QueryConnection,
    timeout_ms: u64,
) -> Result<()> {
    if session.is_none() {
        reconnect_session(session, connection, timeout_ms)
            .context("failed to reconnect before a warmup request")?;
    }
    Ok(())
}

fn query_session(session: &mut Option<TextQuerySession>, request: &str) -> Result<String> {
    session
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("transport session is not open"))?
        .query_line(request)
}

fn reconnect_session(
    session: &mut Option<TextQuerySession>,
    connection: &QueryConnection,
    timeout_ms: u64,
) -> Result<()> {
    session.take();
    *session = Some(open_text_query_session(connection, timeout_ms)?);
    Ok(())
}

fn summarize_request(
    request: &str,
    warmup_failure_count: usize,
    samples: Vec<BenchmarkSample>,
    mut successful_latencies: Vec<f64>,
    last_response: Option<String>,
) -> RequestBenchmark {
    let attempts = samples.len();
    let success_count = samples
        .iter()
        .filter(|sample| sample.outcome == SampleOutcome::Success)
        .count();
    let timeout_count = samples
        .iter()
        .filter(|sample| sample.outcome == SampleOutcome::Timeout)
        .count();
    let error_count = attempts.saturating_sub(success_count + timeout_count);
    let denominator = attempts as f64;
    let rate = |count: usize| {
        if attempts == 0 {
            0.0
        } else {
            count as f64 / denominator
        }
    };

    successful_latencies.sort_by(f64::total_cmp);
    let latency_ms = latency_summary(&successful_latencies);
    RequestBenchmark {
        request: request.to_string(),
        attempts,
        success_count,
        timeout_count,
        error_count,
        warmup_failure_count,
        success_rate: rate(success_count),
        timeout_rate: rate(timeout_count),
        error_rate: rate(error_count),
        latency_ms,
        last_response,
        samples,
    }
}

fn latency_summary(sorted: &[f64]) -> Option<LatencySummary> {
    let (&min, &max) = (sorted.first()?, sorted.last()?);
    Some(LatencySummary {
        min,
        mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p50: percentile(sorted, 0.50),
        p90: percentile(sorted, 0.90),
        p99: percentile(sorted, 0.99),
        max,
    })
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    let rank = quantile * sorted.len().saturating_sub(1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        sorted[lower] + (sorted[upper] - sorted[lower]) * rank.fract()
    }
}

fn is_timeout_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            )
        }) || cause
            .downcast_ref::<instruments::InstrumentError>()
            .is_some_and(instruments::InstrumentError::is_timeout)
    })
}

fn classify_failure(error: &anyhow::Error) -> FailureKind {
    if is_timeout_error(error) {
        return FailureKind::Timeout;
    }
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<io::Error>() {
            match error.kind() {
                io::ErrorKind::Unsupported => return FailureKind::UnsupportedFeature,
                io::ErrorKind::ConnectionRefused
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::AddrInUse
                | io::ErrorKind::AddrNotAvailable
                | io::ErrorKind::UnexpectedEof => return FailureKind::Connection,
                io::ErrorKind::InvalidData => return FailureKind::Protocol,
                _ => {}
            }
        }
        if cause.downcast_ref::<std::string::FromUtf8Error>().is_some() {
            return FailureKind::Protocol;
        }
    }
    let message = format!("{error:#}");
    if message.contains("empty text response") || message.contains("blank text response") {
        FailureKind::Protocol
    } else if message.contains("connection closed before") {
        FailureKind::Connection
    } else {
        FailureKind::Other
    }
}

fn print_human_report(report: &TransportBenchmarkReport) {
    ui::settings_table(
        "Transport Benchmark",
        vec![
            ("connection".to_string(), report.connection.clone()),
            ("protocol".to_string(), report.protocol.to_string()),
            ("timeout".to_string(), format!("{} ms", report.timeout_ms)),
            ("warmup".to_string(), report.warmup.to_string()),
            ("iterations".to_string(), report.iterations.to_string()),
        ],
    );
    println!(
        "{}",
        ui::table(
            &[
                "Request",
                "OK",
                "Timeout",
                "Error",
                "Timeout %",
                "p50 ms",
                "p90 ms",
                "p99 ms",
            ],
            report
                .results
                .iter()
                .map(|result| {
                    vec![
                        result.request.clone(),
                        result.success_count.to_string(),
                        result.timeout_count.to_string(),
                        result.error_count.to_string(),
                        format!("{:.2}", result.timeout_rate * 100.0),
                        format_latency(result.latency_ms.as_ref().map(|stats| stats.p50)),
                        format_latency(result.latency_ms.as_ref().map(|stats| stats.p90)),
                        format_latency(result.latency_ms.as_ref().map(|stats| stats.p99)),
                    ]
                })
                .collect(),
        )
    );
    for result in &report.results {
        if result.timeout_count > 0 || result.error_count > 0 || result.warmup_failure_count > 0 {
            ui::warn(format!(
                "{}: warmup failures={}, timeouts={}, errors={}",
                result.request,
                result.warmup_failure_count,
                result.timeout_count,
                result.error_count
            ));
        }
    }
}

fn print_scpi_query_report(report: &ScpiQueryBenchmarkReport) {
    ui::settings_table(
        "SCPI Query Benchmark",
        vec![
            ("connection".to_string(), report.connection.clone()),
            ("command".to_string(), report.command.clone()),
            ("timeout".to_string(), format!("{} ms", report.timeout_ms)),
            ("warmup".to_string(), report.warmup.to_string()),
            ("iterations".to_string(), report.iterations.to_string()),
        ],
    );
    println!(
        "{}",
        ui::table(
            &[
                "OK", "Timeout", "Error", "min ms", "p50 ms", "p90 ms", "p99 ms", "max ms",
                "query/s",
            ],
            vec![vec![
                report.success_count.to_string(),
                report.timeout_count.to_string(),
                report.error_count.to_string(),
                format_latency(report.min_ms),
                format_latency(report.p50_ms),
                format_latency(report.p90_ms),
                format_latency(report.p99_ms),
                format_latency(report.max_ms),
                format!("{:.2}", report.query_per_sec),
            ]],
        )
    );
    if report.warmup_failure_count > 0 {
        ui::warn(format!("warmup failures: {}", report.warmup_failure_count));
    }
}

fn format_latency(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |value| format!("{value:.3}"))
}

fn protocol_name(protocol: BenchProtocol) -> &'static str {
    match protocol {
        BenchProtocol::Scpi => "scpi",
        BenchProtocol::Line => "line",
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn save_scpi_query_report(
    timestamp: &str,
    contents: &[u8],
    output: Option<&Path>,
    run_dir: Option<&Path>,
    force: bool,
) -> Result<PathBuf> {
    if let Some(output) = output {
        write_report(output, contents, force)?;
        return Ok(output.to_path_buf());
    }

    let directory = run_dir
        .unwrap_or_else(|| Path::new("."))
        .join(SCPI_QUERY_REPORT_DIR);
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create SCPI benchmark directory: {}",
            directory.display()
        )
    })?;
    let timestamp_path = write_timestamped_report(&directory, timestamp, contents)?;
    let latest_path = directory.join("latest.toml");
    write_report(&latest_path, contents, true)?;
    Ok(timestamp_path)
}

fn write_timestamped_report(directory: &Path, timestamp: &str, contents: &[u8]) -> Result<PathBuf> {
    let stamp = filename_timestamp(timestamp)?;
    for sequence in 0..1_000_u16 {
        let file_name = if sequence == 0 {
            format!("{stamp}.toml")
        } else {
            format!("{stamp}-{sequence:03}.toml")
        };
        let path = directory.join(file_name);
        match write_new_report(&path, contents) {
            Ok(()) => return Ok(path),
            Err(error) if error_chain_has_kind(&error, io::ErrorKind::AlreadyExists) => continue,
            Err(error) => return Err(error),
        }
    }
    bail!("failed to allocate a unique SCPI benchmark report name for {stamp}")
}

fn filename_timestamp(timestamp: &str) -> Result<String> {
    let digits = timestamp
        .chars()
        .filter(char::is_ascii_digit)
        .take(14)
        .collect::<String>();
    if digits.len() != 14 {
        bail!("invalid benchmark timestamp: {timestamp}");
    }
    Ok(format!("{}-{}", &digits[..8], &digits[8..]))
}

fn error_chain_has_kind(error: &anyhow::Error, kind: io::ErrorKind) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|error| error.kind() == kind)
}

fn write_report(path: &Path, contents: &[u8], force: bool) -> Result<()> {
    if !force {
        return write_new_report(path, contents);
    }
    let (temporary, mut file) = create_temporary_report(path)?;
    let result = (|| -> Result<()> {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        crate::commands::run_dir::replace_file_atomically(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to save benchmark report: {}", path.display()))
}

fn write_new_report(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(error).with_context(|| {
                format!("refusing to overwrite benchmark report: {}", path.display())
            });
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create benchmark report: {}", path.display()));
        }
    };
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error)
            .with_context(|| format!("failed to save benchmark report: {}", path.display()));
    }
    drop(file);
    crate::commands::run_dir::sync_parent(path)
        .with_context(|| format!("failed to persist benchmark report: {}", path.display()))
}

fn create_temporary_report(path: &Path) -> Result<(PathBuf, fs::File)> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("benchmark output must name a file: {}", path.display()))?;
    for _ in 0..100 {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
        let temporary = path.with_file_name(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temporary report: {}", temporary.display())
                });
            }
        }
    }
    bail!("failed to allocate a temporary benchmark report name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn latency_summary_interpolates_percentiles() {
        let summary = latency_summary(&[1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(summary.min, 1.0);
        assert_eq!(summary.mean, 2.5);
        assert_eq!(summary.p50, 2.5);
        assert!((summary.p90 - 3.7).abs() < 1.0e-12);
        assert!((summary.p99 - 3.97).abs() < 1.0e-12);
        assert_eq!(summary.max, 4.0);
    }

    #[test]
    fn validation_caps_total_stored_samples_before_opening_transport() {
        let allowed_requests = vec!["A?".to_string(), "B?".to_string()];
        let allowed = TransportOptions {
            connection: "tcp://127.0.0.1:1",
            protocol: BenchProtocol::Scpi,
            requests: &allowed_requests,
            iterations: 100_000,
            warmup: 0,
            timeout_ms: 1,
            output: None,
            json: false,
            force: false,
        };
        validate_options(&allowed).unwrap();

        let rejected_requests = vec!["A?".to_string(), "B?".to_string(), "C?".to_string()];
        let rejected = TransportOptions {
            requests: &rejected_requests,
            ..allowed
        };
        let error = validate_options(&rejected).unwrap_err();

        assert!(error.to_string().contains("would store 300000 samples"));
        assert!(error.to_string().contains("at most 200000 total samples"));
    }

    #[test]
    fn transport_benchmark_reuses_connection_and_saves_measured_samples() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..5 {
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, "*IDN?\n");
                reader.get_mut().write_all(b"MOCK,DEVICE,1,0\n").unwrap();
                reader.get_mut().flush().unwrap();
            }
        });
        let output = temporary_path("bench-integration.json");
        let connection = format!("tcp://{address}");
        let requests = vec!["*IDN?".to_string()];

        run_transport(TransportOptions {
            connection: &connection,
            protocol: BenchProtocol::Scpi,
            requests: &requests,
            iterations: 3,
            warmup: 2,
            timeout_ms: 1_000,
            output: Some(&output),
            json: false,
            force: false,
        })
        .unwrap();
        server.join().unwrap();

        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["warmup"], 2);
        assert_eq!(report["results"][0]["attempts"], 3);
        assert_eq!(report["results"][0]["success_count"], 3);
        assert_eq!(report["results"][0]["samples"].as_array().unwrap().len(), 3);
        assert_eq!(report["results"][0]["last_response"], "MOCK,DEVICE,1,0");
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn transport_benchmark_reconnects_after_request_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            let mut first = BufReader::new(first);
            let mut request = String::new();
            first.read_line(&mut request).unwrap();
            assert_eq!(request, "*IDN?\n");
            drop(first);

            let (second, _) = listener.accept().unwrap();
            let mut second = BufReader::new(second);
            request.clear();
            second.read_line(&mut request).unwrap();
            assert_eq!(request, "*IDN?\n");
            second.get_mut().write_all(b"MOCK,RECOVERED,1,0\n").unwrap();
            second.get_mut().flush().unwrap();
        });
        let output = temporary_path("bench-reconnect.json");
        let connection = format!("tcp://{address}");
        let requests = vec!["*IDN?".to_string()];

        run_transport(TransportOptions {
            connection: &connection,
            protocol: BenchProtocol::Scpi,
            requests: &requests,
            iterations: 2,
            warmup: 0,
            timeout_ms: 1_000,
            output: Some(&output),
            json: false,
            force: false,
        })
        .unwrap();
        server.join().unwrap();

        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["results"][0]["success_count"], 1);
        assert_eq!(report["results"][0]["error_count"], 1);
        assert_eq!(report["results"][0]["last_response"], "MOCK,RECOVERED,1,0");
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn transport_benchmark_reconnects_between_requests_without_warmup() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            let mut first = BufReader::new(first);
            let mut request = String::new();
            first.read_line(&mut request).unwrap();
            assert_eq!(request, "FIRST?\n");
            drop(first);

            let (second, _) = listener.accept().unwrap();
            let mut second = BufReader::new(second);
            request.clear();
            second.read_line(&mut request).unwrap();
            assert_eq!(request, "SECOND?\n");
            second.get_mut().write_all(b"RECOVERED\n").unwrap();
            second.get_mut().flush().unwrap();
        });
        let output = temporary_path("bench-request-boundary.json");
        let connection = format!("tcp://{address}");
        let requests = vec!["FIRST?".to_string(), "SECOND?".to_string()];

        run_transport(TransportOptions {
            connection: &connection,
            protocol: BenchProtocol::Scpi,
            requests: &requests,
            iterations: 1,
            warmup: 0,
            timeout_ms: 1_000,
            output: Some(&output),
            json: false,
            force: false,
        })
        .unwrap();
        server.join().unwrap();

        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["results"][0]["error_count"], 1);
        assert_eq!(report["results"][1]["success_count"], 1);
        assert_eq!(report["results"][1]["last_response"], "RECOVERED");
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn transport_benchmark_saves_a_report_when_every_request_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "*IDN?\n");
            thread::sleep(Duration::from_millis(100));
        });
        let output = temporary_path("bench-all-timeout.json");
        let connection = format!("tcp://{address}");
        let requests = vec!["*IDN?".to_string()];

        let error = run_transport(TransportOptions {
            connection: &connection,
            protocol: BenchProtocol::Scpi,
            requests: &requests,
            iterations: 1,
            warmup: 0,
            timeout_ms: 20,
            output: Some(&output),
            json: false,
            force: false,
        })
        .unwrap_err();
        server.join().unwrap();

        assert!(error.to_string().contains("without a successful response"));
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["results"][0]["success_count"], 0);
        assert_eq!(report["results"][0]["timeout_count"], 1);
        assert_eq!(report["results"][0]["error_count"], 0);
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn transport_benchmark_records_reconnect_failures_and_finishes_report() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(listener);
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "*IDN?\n");
            thread::sleep(Duration::from_millis(100));
        });
        let output = temporary_path("bench-reconnect-failure.json");
        let connection = format!("tcp://{address}");
        let requests = vec!["*IDN?".to_string()];

        let error = run_transport(TransportOptions {
            connection: &connection,
            protocol: BenchProtocol::Scpi,
            requests: &requests,
            iterations: 2,
            warmup: 0,
            timeout_ms: 20,
            output: Some(&output),
            json: false,
            force: false,
        })
        .unwrap_err();
        server.join().unwrap();

        assert!(error.to_string().contains("without a successful response"));
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["results"][0]["attempts"], 2);
        assert_eq!(report["results"][0]["success_count"], 0);
        let timeout_count = report["results"][0]["timeout_count"].as_u64().unwrap();
        let error_count = report["results"][0]["error_count"].as_u64().unwrap();
        assert_eq!(timeout_count + error_count, 2);
        assert!(
            report["results"][0]["samples"][1]["error"]
                .as_str()
                .unwrap()
                .contains("failed to reconnect")
        );
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn scpi_query_benchmark_saves_timestamped_toml_and_regular_latest_copy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            for _ in 0..3 {
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, "*IDN?\n");
                reader.get_mut().write_all(b"MOCK,SCPI,1,0\n").unwrap();
                reader.get_mut().flush().unwrap();
            }
        });
        let run_dir = temporary_path("scpi-query-run");
        let connection = format!("tcp://{address}");

        run_scpi_query(ScpiQueryOptions {
            connection: &connection,
            command: "*IDN?",
            iterations: 2,
            warmup: 1,
            timeout_ms: 1_000,
            output: None,
            run_dir: Some(&run_dir),
            json: false,
            force: false,
        })
        .unwrap();
        server.join().unwrap();

        let directory = run_dir.join(SCPI_QUERY_REPORT_DIR);
        let latest = directory.join("latest.toml");
        assert!(latest.is_file());
        assert!(
            !fs::symlink_metadata(&latest)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let report_paths = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(report_paths.len(), 2);
        let timestamped = report_paths
            .into_iter()
            .find(|path| path.file_name().unwrap() != "latest.toml")
            .unwrap();
        let latest_bytes = fs::read(&latest).unwrap();
        assert_eq!(latest_bytes, fs::read(timestamped).unwrap());
        let report: toml::Value = toml::from_slice(&latest_bytes).unwrap();
        assert_eq!(report["schema_version"].as_integer(), Some(1));
        assert_eq!(
            report["pmoke_version"].as_str(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert!(report["git_commit"].as_str().is_some());
        assert!(report["timestamp"].as_str().is_some());
        assert_eq!(report["os"].as_str(), Some(env::consts::OS));
        assert_eq!(report["arch"].as_str(), Some(env::consts::ARCH));
        assert_eq!(report["protocol"].as_str(), Some("scpi"));
        assert_eq!(report["warmup"].as_integer(), Some(1));
        assert_eq!(report["iterations"].as_integer(), Some(2));
        assert_eq!(report["success_count"].as_integer(), Some(2));
        assert_eq!(report["last_response"].as_str(), Some("MOCK,SCPI,1,0"));
        assert!(report["min_ms"].as_float().is_some());
        assert!(report["mean_ms"].as_float().is_some());
        assert!(report["p99_ms"].as_float().is_some());
        assert!(report["query_per_sec"].as_float().unwrap() > 0.0);
        for kind in FailureKind::ALL {
            assert_eq!(
                report["errors_by_kind"][kind.as_str()].as_integer(),
                Some(0)
            );
        }
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn scpi_query_benchmark_records_all_timeout_before_returning_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "*IDN?\n");
            thread::sleep(Duration::from_millis(100));
        });
        let run_dir = temporary_path("scpi-query-timeout");
        let connection = format!("tcp://{address}");

        let error = run_scpi_query(ScpiQueryOptions {
            connection: &connection,
            command: "*IDN?",
            iterations: 1,
            warmup: 0,
            timeout_ms: 20,
            output: None,
            run_dir: Some(&run_dir),
            json: false,
            force: false,
        })
        .unwrap_err();
        server.join().unwrap();

        assert!(error.to_string().contains("without a successful response"));
        let report: toml::Value = toml::from_str(
            &fs::read_to_string(run_dir.join(SCPI_QUERY_REPORT_DIR).join("latest.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["success_count"].as_integer(), Some(0));
        assert_eq!(report["timeout_count"].as_integer(), Some(1));
        assert_eq!(report["errors_by_kind"]["timeout"].as_integer(), Some(1));
        assert!(report.get("p50_ms").is_none());
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn timestamped_report_uses_a_collision_suffix() {
        let run_dir = temporary_path("scpi-query-collision");
        let directory = run_dir.join(SCPI_QUERY_REPORT_DIR);
        fs::create_dir_all(&directory).unwrap();
        let timestamp = "2026-08-01T13:30:45.123456789Z";

        let first = write_timestamped_report(&directory, timestamp, b"first").unwrap();
        let second = write_timestamped_report(&directory, timestamp, b"second").unwrap();

        assert_eq!(first.file_name().unwrap(), "20260801-133045.toml");
        assert_eq!(second.file_name().unwrap(), "20260801-133045-001.toml");
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn failure_classification_distinguishes_actionable_categories() {
        let timeout = anyhow::Error::new(io::Error::new(io::ErrorKind::TimedOut, "late"));
        let connection =
            anyhow::Error::new(io::Error::new(io::ErrorKind::ConnectionRefused, "closed"));
        let unsupported = anyhow::Error::new(io::Error::new(
            io::ErrorKind::Unsupported,
            "feature disabled",
        ));
        let protocol = anyhow::Error::new(io::Error::new(io::ErrorKind::InvalidData, "bad"));

        assert_eq!(classify_failure(&timeout), FailureKind::Timeout);
        assert_eq!(classify_failure(&connection), FailureKind::Connection);
        assert_eq!(
            classify_failure(&unsupported),
            FailureKind::UnsupportedFeature
        );
        assert_eq!(classify_failure(&protocol), FailureKind::Protocol);
        assert_eq!(
            classify_failure(&anyhow::anyhow!("unknown")),
            FailureKind::Other
        );
    }

    #[test]
    fn request_summary_excludes_failures_from_latency() {
        let samples = vec![
            BenchmarkSample {
                iteration: 1,
                outcome: SampleOutcome::Success,
                elapsed_ms: 2.0,
                error: None,
                failure_kind: None,
            },
            BenchmarkSample {
                iteration: 2,
                outcome: SampleOutcome::Timeout,
                elapsed_ms: 10.0,
                error: Some("timeout".to_string()),
                failure_kind: Some(FailureKind::Timeout),
            },
            BenchmarkSample {
                iteration: 3,
                outcome: SampleOutcome::Error,
                elapsed_ms: 1.0,
                error: Some("error".to_string()),
                failure_kind: Some(FailureKind::Other),
            },
        ];
        let result = summarize_request("*IDN?", 1, samples, vec![2.0], Some("IDN".to_string()));

        assert_eq!(result.success_count, 1);
        assert_eq!(result.timeout_count, 1);
        assert_eq!(result.error_count, 1);
        assert_eq!(result.timeout_rate, 1.0 / 3.0);
        assert_eq!(result.latency_ms.unwrap().mean, 2.0);
    }

    #[test]
    fn report_writer_refuses_overwrite_without_force() {
        let path = temporary_path("bench-report.json");
        fs::write(&path, b"old").unwrap();

        let error = write_report(&path, b"new", false).unwrap_err();

        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn report_writer_distinguishes_create_failure_from_overwrite() {
        let path = temporary_path("missing-parent").join("bench-report.json");

        let error = write_report(&path, b"new", false).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to create benchmark report")
        );
        assert!(!error.to_string().contains("refusing to overwrite"));
    }

    #[test]
    fn report_writer_replaces_with_force() {
        let path = temporary_path("bench-report-force.json");
        fs::write(&path, b"old").unwrap();

        write_report(&path, b"new", true).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        fs::remove_file(path).unwrap();
    }

    fn temporary_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pmoke-{name}-{}-{nonce}", std::process::id()))
    }
}
