use crate::cli::{InstrumentsCommand, JsonOutput};
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use instruments::registry::{InstrumentSpec, KNOWN_INSTRUMENTS};
use instruments::transport::{ScpiConnection, open_scpi_transport};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const DEFAULT_PROLOGIX_PORT: u16 = 1234;
const DEFAULT_PROLOGIX_BAUD_RATE: u32 = 115_200;

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InstrumentListItem {
    model: &'static str,
    role: &'static str,
    transports: Vec<&'static str>,
    protocols: Vec<&'static str>,
    capabilities: Vec<&'static str>,
    required_features: Vec<&'static str>,
    notes: Vec<&'static str>,
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InstrumentDetails {
    model: &'static str,
    role: &'static str,
    transports: Vec<&'static str>,
    protocols: Vec<&'static str>,
    capabilities: Vec<&'static str>,
    notes: Vec<&'static str>,
    connection_templates: Vec<ConnectionTemplateDetails>,
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ConnectionTemplateDetails {
    transport: &'static str,
    connection_template: &'static str,
    required_feature: Option<&'static str>,
    feature_note: Option<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct QueryOutput {
    connection: String,
    command: String,
    response: String,
}

#[derive(Debug, Clone)]
enum QueryConnection {
    Tcpip { host: String, port: u16 },
    Scpi(ScpiConnection),
}

pub fn run(command: &InstrumentsCommand) -> Result<()> {
    match command {
        InstrumentsCommand::List(JsonOutput { json }) => list(*json),
        InstrumentsCommand::Explain { model, json } => explain(model, *json),
        InstrumentsCommand::Query {
            connection,
            timeout_ms,
            json,
            command,
        } => query(connection, command, *timeout_ms, *json),
    }
}

fn list(json: bool) -> Result<()> {
    let instruments = KNOWN_INSTRUMENTS.iter().map(list_item).collect::<Vec<_>>();
    if json {
        println!("{}", serde_json::to_string_pretty(&instruments)?);
        return Ok(());
    }

    ui::section("Supported Instruments");
    println!(
        "{}",
        ui::table(
            &[
                "Model",
                "Role",
                "Transports",
                "Protocols",
                "Features",
                "Notes"
            ],
            instruments
                .iter()
                .map(|item| {
                    vec![
                        item.model.to_string(),
                        item.role.to_string(),
                        item.transports.join(", "),
                        item.protocols.join(", "),
                        display_list(&item.required_features),
                        display_list(&item.notes),
                    ]
                })
                .collect(),
        )
    );
    Ok(())
}

fn explain(model: &str, json: bool) -> Result<()> {
    let Some(spec) = find_instrument_fuzzy(model) else {
        let models = KNOWN_INSTRUMENTS
            .iter()
            .map(|spec| spec.model)
            .collect::<Vec<_>>()
            .join(", ");
        bail!("unknown instrument model '{model}'. Known models: {models}");
    };
    let details = details(spec);
    if json {
        println!("{}", serde_json::to_string_pretty(&details)?);
        return Ok(());
    }

    ui::settings_table(
        details.model,
        vec![
            ("role".to_string(), details.role.to_string()),
            ("description".to_string(), details.description.to_string()),
            ("transports".to_string(), details.transports.join(", ")),
            ("protocols".to_string(), details.protocols.join(", ")),
            ("capabilities".to_string(), details.capabilities.join(", ")),
            ("notes".to_string(), display_list(&details.notes)),
        ],
    );
    if !details.connection_templates.is_empty() {
        ui::section("Connection Templates");
        println!(
            "{}",
            ui::table(
                &["Transport", "Connection template", "Feature", "Note"],
                details
                    .connection_templates
                    .iter()
                    .map(|example| {
                        vec![
                            example.transport.to_string(),
                            example.connection_template.to_string(),
                            example.required_feature.unwrap_or("-").to_string(),
                            example.feature_note.unwrap_or("-").to_string(),
                        ]
                    })
                    .collect(),
            )
        );
    }
    Ok(())
}

fn query(connection: &str, command: &str, timeout_ms: u64, json: bool) -> Result<()> {
    validate_scpi_query_command(command)?;
    let connection = parse_query_connection(connection, timeout_ms)?;
    let response = run_scpi_text_query(&connection, command, timeout_ms)?;
    let output = QueryOutput {
        connection: display_query_connection(&connection),
        command: command.to_string(),
        response,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", output.response);
    }
    Ok(())
}

fn validate_scpi_query_command(command: &str) -> Result<()> {
    if command.trim().is_empty() {
        bail!("SCPI query command must not be empty");
    }
    if command.contains(['\r', '\n']) {
        bail!("SCPI query command must be a single line");
    }

    let mut quote = None;
    let mut has_query_marker = false;
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                if chars.peek() == Some(&delimiter) {
                    chars.next();
                } else {
                    quote = None;
                }
            }
            continue;
        }

        match character {
            '\'' | '"' => quote = Some(character),
            ';' => bail!("SCPI query command must contain exactly one command"),
            '?' => has_query_marker = true,
            _ => {}
        }
    }
    if quote.is_some() {
        bail!("SCPI query command contains an unterminated quoted string");
    }
    if !has_query_marker {
        bail!("SCPI query command must contain '?'");
    }
    Ok(())
}

fn find_instrument_fuzzy(model: &str) -> Option<&'static InstrumentSpec> {
    instruments::registry::find_instrument(model).or_else(|| {
        KNOWN_INSTRUMENTS
            .iter()
            .find(|spec| spec.model.eq_ignore_ascii_case(model))
    })
}

fn list_item(spec: &InstrumentSpec) -> InstrumentListItem {
    InstrumentListItem {
        model: spec.model,
        role: spec.role.as_str(),
        transports: transport_names(spec),
        protocols: protocol_names(spec),
        capabilities: capability_names(spec),
        required_features: required_features(spec),
        notes: transport_notes(spec),
        description: spec.description,
    }
}

fn details(spec: &InstrumentSpec) -> InstrumentDetails {
    InstrumentDetails {
        model: spec.model,
        role: spec.role.as_str(),
        transports: transport_names(spec),
        protocols: protocol_names(spec),
        capabilities: capability_names(spec),
        notes: transport_notes(spec),
        connection_templates: connection_templates(spec),
        description: spec.description,
    }
}

fn transport_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .map(|transport| transport.as_str())
        .collect()
}

fn protocol_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.protocols
        .iter()
        .map(|protocol| protocol.as_str())
        .collect()
}

fn capability_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect()
}

fn connection_templates(spec: &InstrumentSpec) -> Vec<ConnectionTemplateDetails> {
    spec.transports
        .iter()
        .map(|transport| ConnectionTemplateDetails {
            transport: transport.as_str(),
            connection_template: transport.connection_template(),
            required_feature: transport.required_feature(),
            feature_note: transport.feature_note(),
        })
        .collect()
}

fn required_features(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .filter_map(|transport| transport.required_feature())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn transport_notes(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .filter_map(|transport| transport.feature_note())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn display_list(values: &[&str]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn run_scpi_text_query(
    connection: &QueryConnection,
    command: &str,
    timeout_ms: u64,
) -> Result<String> {
    match connection {
        QueryConnection::Tcpip { host, port } => query_tcp_text(host, *port, command, timeout_ms),
        QueryConnection::Scpi(connection) => {
            let mut transport = open_scpi_transport(connection)
                .with_context(|| format!("failed to open {connection}"))?;
            let response = transport
                .query_line(command)
                .with_context(|| format!("failed to query {connection}"))?;
            let response = trim_scpi_line_ending(&response);
            if response.is_empty() {
                bail!("received an empty SCPI response from {connection}");
            }
            Ok(response.to_string())
        }
    }
}

fn trim_scpi_line_ending(response: &str) -> &str {
    response.trim_end_matches(['\r', '\n'])
}

fn query_tcp_text(host: &str, port: u16, command: &str, timeout_ms: u64) -> Result<String> {
    let timeout = Duration::from_millis(timeout_ms);
    let addresses = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {host}:{port}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        bail!("no socket address resolved for {host}:{port}");
    }

    let mut last_error = None;
    let mut stream = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(candidate) => {
                stream = Some(candidate);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let stream = stream.ok_or_else(|| {
        last_error
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow!("no socket address resolved for {host}:{port}"))
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_nodelay(true)?;

    let mut reader = BufReader::new(stream);
    writeln!(reader.get_mut(), "{command}")?;
    reader.get_mut().flush()?;

    let mut response = String::new();
    for _ in 0..4 {
        response.clear();
        let read = reader.read_line(&mut response)?;
        let line = trim_scpi_line_ending(&response);
        if read == 0 {
            bail!("connection closed before {host}:{port} returned a SCPI response");
        }
        if !line.is_empty() {
            return Ok(line.to_string());
        }
    }
    bail!("received only blank SCPI response lines from {host}:{port}")
}

fn parse_query_connection(value: &str, default_timeout_ms: u64) -> Result<QueryConnection> {
    if default_timeout_ms == 0 {
        bail!("timeout_ms must be positive");
    }
    if let Some(endpoint) = value.strip_prefix("tcp://") {
        let (host, port) = parse_host_port(endpoint, "TCP")?;
        return Ok(QueryConnection::Tcpip { host, port });
    }
    if value.starts_with("visa:") {
        bail!(
            "visa connections are not supported by generic instruments query yet; use a TCP/IP, GPIB, or Prologix SCPI connection"
        );
    }
    if let Some(endpoint) = value.strip_prefix("gpib://") {
        let (board, address) = endpoint
            .split_once('/')
            .ok_or_else(|| anyhow!("GPIB connection must be gpib://board/address"))?;
        let board = parse_u8(board, "GPIB board")?;
        let address = parse_gpib_address(address, "GPIB address")?;
        return Ok(QueryConnection::Scpi(ScpiConnection::Gpib {
            board: i32::from(board),
            address: i32::from(address),
            timeout_secs: timeout_ms_to_secs(default_timeout_ms),
            use_crlf: false,
        }));
    }
    if let Some(endpoint) = value.strip_prefix("prologix-tcp://") {
        let (endpoint, query) = split_query(endpoint);
        let params = parse_query_params(query)?;
        validate_query_param_keys(&params, &["addr", "address", "read_timeout_ms"])?;
        let address = parse_required_address_param(&params)?;
        let read_timeout_ms = parse_timeout_param(&params, default_timeout_ms)?;
        let (host, port) =
            parse_optional_host_port(endpoint, DEFAULT_PROLOGIX_PORT, "Prologix TCP")?;
        return Ok(QueryConnection::Scpi(ScpiConnection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        }));
    }
    if let Some(endpoint) = value.strip_prefix("prologix-serial://") {
        let (path, query) = split_query(endpoint);
        let params = parse_query_params(query)?;
        validate_query_param_keys(
            &params,
            &["addr", "address", "baud_rate", "read_timeout_ms"],
        )?;
        let address = parse_required_address_param(&params)?;
        let read_timeout_ms = parse_timeout_param(&params, default_timeout_ms)?;
        let baud_rate = parse_optional_u32_param(&params, "baud_rate", DEFAULT_PROLOGIX_BAUD_RATE)?;
        if baud_rate == 0 {
            bail!("Prologix serial baud_rate must be positive");
        }
        let path = path.trim();
        if path.is_empty() {
            bail!("Prologix serial path must not be empty");
        }
        return Ok(QueryConnection::Scpi(ScpiConnection::PrologixSerial {
            path: path.to_string(),
            address,
            baud_rate,
            read_timeout_ms,
        }));
    }

    bail!(
        "unsupported connection URI '{value}'; use tcp://host:port, gpib://board/address, prologix-tcp://host[:port]?addr=address, or prologix-serial:///path?addr=address"
    )
}

fn display_query_connection(connection: &QueryConnection) -> String {
    match connection {
        QueryConnection::Tcpip { host, port } if host.contains(':') => {
            format!("tcp://[{host}]:{port}")
        }
        QueryConnection::Tcpip { host, port } => format!("tcp://{host}:{port}"),
        QueryConnection::Scpi(ScpiConnection::Gpib { board, address, .. }) => {
            format!("gpib://{board}/{address}")
        }
        QueryConnection::Scpi(ScpiConnection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        }) if host.contains(':') => {
            format!(
                "prologix-tcp://[{host}]:{port}?addr={address}&read_timeout_ms={read_timeout_ms}"
            )
        }
        QueryConnection::Scpi(ScpiConnection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        }) => {
            format!("prologix-tcp://{host}:{port}?addr={address}&read_timeout_ms={read_timeout_ms}")
        }
        QueryConnection::Scpi(ScpiConnection::PrologixSerial {
            path,
            address,
            baud_rate,
            read_timeout_ms,
        }) => {
            format!(
                "prologix-serial://{path}?addr={address}&baud_rate={baud_rate}&read_timeout_ms={read_timeout_ms}"
            )
        }
    }
}

fn parse_host_port(endpoint: &str, label: &str) -> Result<(String, u16)> {
    parse_optional_host_port(endpoint, 0, label)
}

fn parse_optional_host_port(
    endpoint: &str,
    default_port: u16,
    label: &str,
) -> Result<(String, u16)> {
    let (host, port) = if let Some(rest) = endpoint.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:") {
            (
                host,
                port.parse::<u16>()
                    .with_context(|| format!("invalid {label} port: {port}"))?,
            )
        } else if let Some(host) = rest.strip_suffix(']') {
            (host, default_port)
        } else {
            bail!("{label} IPv6 endpoint must be [address]:port or [address]");
        }
    } else if let Some((host, port)) = endpoint.rsplit_once(':') {
        if host.contains(':') {
            bail!("{label} IPv6 endpoint must use [address]:port or [address]");
        }
        (
            host,
            port.parse::<u16>()
                .with_context(|| format!("invalid {label} port: {port}"))?,
        )
    } else {
        (endpoint, default_port)
    };
    let host = host.trim();
    if host.is_empty() {
        bail!("{label} host must not be empty");
    }
    if port == 0 {
        bail!("{label} endpoint must include a port");
    }
    Ok((host.to_string(), port))
}

fn split_query(value: &str) -> (&str, Option<&str>) {
    match value.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (value, None),
    }
}

fn parse_query_params(query: Option<&str>) -> Result<Vec<(&str, &str)>> {
    let Some(query) = query else {
        return Ok(Vec::new());
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once('=')
                .ok_or_else(|| anyhow!("query parameter '{part}' must be key=value"))
        })
        .collect()
}

fn validate_query_param_keys(params: &[(&str, &str)], allowed: &[&str]) -> Result<()> {
    for (key, _) in params {
        if !allowed.contains(key) {
            bail!("unsupported query parameter: {key}");
        }
    }
    Ok(())
}

fn parse_required_address_param(params: &[(&str, &str)]) -> Result<u8> {
    let address = find_unique_query_param(params, &["addr", "address"], "Prologix address")?
        .ok_or_else(|| anyhow!("Prologix connection must include addr=0..30"))?;
    parse_gpib_address(address, "Prologix address")
}

fn parse_timeout_param(params: &[(&str, &str)], default_timeout_ms: u64) -> Result<u16> {
    let timeout = match find_unique_query_param(params, &["read_timeout_ms"], "read_timeout_ms")? {
        Some(value) => value
            .parse::<u64>()
            .with_context(|| format!("invalid read_timeout_ms: {value}"))?,
        None => default_timeout_ms,
    };
    if !(1..=3000).contains(&timeout) {
        bail!("Prologix read_timeout_ms must be in 1..=3000 (got {timeout})");
    }
    Ok(timeout as u16)
}

fn parse_optional_u32_param(params: &[(&str, &str)], key: &str, default: u32) -> Result<u32> {
    let Some(value) = find_unique_query_param(params, &[key], key)? else {
        return Ok(default);
    };
    value
        .parse::<u32>()
        .with_context(|| format!("invalid {key}: {value}"))
}

fn find_unique_query_param<'a>(
    params: &[(&str, &'a str)],
    keys: &[&str],
    label: &str,
) -> Result<Option<&'a str>> {
    let mut values = params
        .iter()
        .filter_map(|(key, value)| keys.contains(key).then_some(*value));
    let value = values.next();
    if values.next().is_some() {
        bail!("{label} must be specified only once");
    }
    Ok(value)
}

fn parse_gpib_address(value: &str, label: &str) -> Result<u8> {
    let address = parse_u8(value, label)?;
    if address > 30 {
        bail!("{label} must be in 0..=30 (got {address})");
    }
    Ok(address)
}

fn parse_u8(value: &str, label: &str) -> Result<u8> {
    value
        .parse::<u8>()
        .with_context(|| format!("invalid {label}: {value}"))
}

fn timeout_ms_to_secs(timeout_ms: u64) -> u64 {
    timeout_ms.div_ceil(1000).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn list_includes_registry_metadata() {
        let item = list_item(instruments::registry::find_instrument("Keithley2010").unwrap());

        assert_eq!(item.role, "multimeter");
        assert!(item.transports.contains(&"prologix_tcp"));
        assert!(item.protocols.contains(&"scpi"));
        assert!(item.capabilities.contains(&"scpi_identify"));
    }

    #[test]
    fn explain_accepts_case_insensitive_model_names() {
        let spec = find_instrument_fuzzy("keithley2010").unwrap();

        assert_eq!(spec.model, "Keithley2010");
    }

    #[test]
    fn details_generate_connection_templates_from_transports() {
        let spec = instruments::registry::find_instrument("Keithley2010").unwrap();
        let details = details(spec);

        assert!(details.connection_templates.iter().any(|example| {
            example.transport == "prologix_tcp"
                && example.connection_template == "prologix-tcp://<host>:1234?addr=<addr>"
                && example.required_feature == Some("hw-prologix-tcp")
        }));
    }

    #[test]
    fn usbtmc_notes_are_separate_from_feature_names() {
        let spec = instruments::registry::find_instrument("DHO5108").unwrap();
        let item = list_item(spec);
        let details = details(spec);

        assert!(item.required_features.contains(&"hw-gpib"));
        assert!(!item.required_features.contains(&"hw-gpib on Windows"));
        assert_eq!(item.notes, vec!["Windows + NI-VISA"]);
        assert!(details.connection_templates.iter().any(|example| {
            example.transport == "usbtmc"
                && example.required_feature == Some("hw-gpib")
                && example.feature_note == Some("Windows + NI-VISA")
        }));
    }

    #[test]
    fn dummy_instrument_has_no_required_feature_name() {
        let spec = instruments::registry::find_instrument("DummyInstrument").unwrap();
        let item = list_item(spec);
        let details = details(spec);

        assert!(item.required_features.is_empty());
        assert_eq!(display_list(&item.required_features), "-");
        assert!(
            details.connection_templates.iter().any(|example| {
                example.transport == "dummy" && example.required_feature.is_none()
            })
        );
    }

    #[test]
    fn query_connection_parser_accepts_tcpip_and_prologix_tcp() {
        let tcp = parse_query_connection("tcp://192.168.10.100:55255", 2500).unwrap();
        assert!(matches!(
            tcp,
            QueryConnection::Tcpip { host, port }
                if host == "192.168.10.100" && port == 55255
        ));

        let tcp_ipv6 = parse_query_connection("tcp://[2001:db8::1]:55255", 2500).unwrap();
        assert!(matches!(
            tcp_ipv6,
            QueryConnection::Tcpip { host, port }
                if host == "2001:db8::1" && port == 55255
        ));

        let prologix =
            parse_query_connection("prologix-tcp://10.249.11.17:1234?addr=17", 2500).unwrap();
        assert!(matches!(
            prologix,
            QueryConnection::Scpi(ScpiConnection::PrologixTcp {
                ref host,
                port: 1234,
                address: 17,
                read_timeout_ms: 2500,
            }) if host == "10.249.11.17"
        ));
        assert_eq!(
            display_query_connection(&prologix),
            "prologix-tcp://10.249.11.17:1234?addr=17&read_timeout_ms=2500"
        );
    }

    #[test]
    fn query_connection_parser_rejects_non_scpi_and_invalid_prologix_timeout() {
        let visa = parse_query_connection("visa:USB0::1234::INSTR", 2500).unwrap_err();
        assert!(visa.to_string().contains("not supported"));

        let zero_timeout = parse_query_connection("tcp://host:1234", 0).unwrap_err();
        assert!(zero_timeout.to_string().contains("must be positive"));

        let ipv6 = parse_query_connection("tcp://2001:db8::1:55255", 2500).unwrap_err();
        assert!(ipv6.to_string().contains("IPv6 endpoint must use"));

        let timeout = parse_query_connection(
            "prologix-tcp://host:1234?addr=17&read_timeout_ms=5000",
            2500,
        )
        .unwrap_err();
        assert!(
            timeout
                .to_string()
                .contains("read_timeout_ms must be in 1..=3000")
        );

        let duplicate_address =
            parse_query_connection("prologix-tcp://host:1234?addr=17&address=18", 2500)
                .unwrap_err();
        assert!(
            duplicate_address
                .to_string()
                .contains("address must be specified only once")
        );

        let duplicate_timeout = parse_query_connection(
            "prologix-tcp://host:1234?addr=17&read_timeout_ms=10&read_timeout_ms=20",
            2500,
        )
        .unwrap_err();
        assert!(
            duplicate_timeout
                .to_string()
                .contains("read_timeout_ms must be specified only once")
        );
    }

    #[test]
    fn query_connection_parser_accepts_gpib_and_prologix_serial() {
        let gpib = parse_query_connection("gpib://0/17", 2500).unwrap();
        assert!(matches!(
            gpib,
            QueryConnection::Scpi(ScpiConnection::Gpib {
                board: 0,
                address: 17,
                timeout_secs: 3,
                use_crlf: false,
            })
        ));

        let serial = parse_query_connection(
            "prologix-serial:///dev/cu.usbserial-XXXX?addr=11&baud_rate=57600",
            1500,
        )
        .unwrap();
        assert!(matches!(
            serial,
            QueryConnection::Scpi(ScpiConnection::PrologixSerial {
                path,
                address: 11,
                baud_rate: 57600,
                read_timeout_ms: 1500,
            }) if path == "/dev/cu.usbserial-XXXX"
        ));
    }

    #[test]
    fn tcp_query_writes_command_and_preserves_payload_whitespace() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            assert_eq!(command, "*IDN?\n");
            write!(reader.get_mut(), "  MOCK,MODEL,SERIAL,FIRMWARE  \r\n").unwrap();
            reader.get_mut().flush().unwrap();
        });

        let response = query_tcp_text("127.0.0.1", port, "*IDN?", 1000).unwrap();

        server.join().unwrap();
        assert_eq!(response, "  MOCK,MODEL,SERIAL,FIRMWARE  ");
    }

    #[test]
    fn tcp_query_rejects_eof_before_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            assert_eq!(command, "*IDN?\n");
        });

        let error = query_tcp_text("127.0.0.1", port, "*IDN?", 1000).unwrap_err();

        server.join().unwrap();
        assert!(error.to_string().contains("closed before"));
    }

    #[test]
    fn scpi_response_trimming_only_removes_line_endings() {
        assert_eq!(trim_scpi_line_ending("  +1.0  \r\n"), "  +1.0  ");
        assert_eq!(trim_scpi_line_ending("  +1.0  "), "  +1.0  ");
        assert_eq!(trim_scpi_line_ending("\r\n"), "");
    }

    #[test]
    fn scpi_query_validation_accepts_one_query() {
        assert!(validate_scpi_query_command("*IDN?").is_ok());
        assert!(validate_scpi_query_command(":SYST:ERR? 'A;B'").is_ok());
        assert!(validate_scpi_query_command(":SYST:ERR? 'A''B'").is_ok());
    }

    #[test]
    fn scpi_query_validation_rejects_writes_and_multiple_commands() {
        let write_after_query = validate_scpi_query_command("*IDN?;*RST").unwrap_err();
        assert!(
            write_after_query
                .to_string()
                .contains("exactly one command")
        );

        let multiple_queries = validate_scpi_query_command(":SYST:ERR?;:STAT?").unwrap_err();
        assert!(multiple_queries.to_string().contains("exactly one command"));

        let empty = validate_scpi_query_command("  ").unwrap_err();
        assert!(empty.to_string().contains("must not be empty"));

        let write = validate_scpi_query_command("*RST").unwrap_err();
        assert!(write.to_string().contains("must contain '?'"));

        let quoted_marker = validate_scpi_query_command(":DISP:TEXT '?'").unwrap_err();
        assert!(quoted_marker.to_string().contains("must contain '?'"));

        let unterminated = validate_scpi_query_command(":SYST:ERR? '").unwrap_err();
        assert!(unterminated.to_string().contains("unterminated"));

        let multiline = validate_scpi_query_command("*IDN?\n++addr 5").unwrap_err();
        assert!(multiline.to_string().contains("must be a single line"));
    }
}
