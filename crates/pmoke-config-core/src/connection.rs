use std::fmt;

pub const DEFAULT_PROLOGIX_PORT: u16 = 1234;
pub const DEFAULT_PROLOGIX_BAUD_RATE: u32 = 115_200;
pub const DEFAULT_PROLOGIX_READ_TIMEOUT_MS: u16 = 3000;

#[derive(Debug, Clone, Copy)]
pub struct ConnectionDefaults {
    pub prologix_read_timeout_ms: u64,
}

impl Default for ConnectionDefaults {
    fn default() -> Self {
        Self {
            prologix_read_timeout_ms: u64::from(DEFAULT_PROLOGIX_READ_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionUri {
    Tcp {
        host: String,
        port: u16,
    },
    Visa {
        resource: String,
    },
    Gpib {
        board: u8,
        address: u8,
    },
    PrologixTcp {
        host: String,
        port: u16,
        address: u8,
        read_timeout_ms: u16,
    },
    PrologixSerial {
        path: String,
        address: u8,
        baud_rate: u32,
        read_timeout_ms: u16,
    },
}

impl ConnectionUri {
    pub fn parse(value: &str, defaults: ConnectionDefaults) -> Result<Self, String> {
        if let Some(endpoint) = value.strip_prefix("tcp://") {
            let (host, port) = parse_host_port(endpoint, None, "TCP")?;
            return Ok(Self::Tcp { host, port });
        }
        if let Some(resource) = value.strip_prefix("visa:") {
            let resource = resource.trim();
            if resource.is_empty() {
                return Err("VISA resource must not be empty".to_string());
            }
            return Ok(Self::Visa {
                resource: resource.to_string(),
            });
        }
        if let Some(endpoint) = value.strip_prefix("gpib://") {
            let (board, address) = endpoint
                .split_once('/')
                .ok_or_else(|| "GPIB connection must be gpib://board/address".to_string())?;
            return Ok(Self::Gpib {
                board: parse_u8(board, "GPIB board")?,
                address: parse_gpib_address(address, "GPIB address")?,
            });
        }
        if let Some(endpoint) = value.strip_prefix("prologix-tcp://") {
            let (endpoint, query) = split_query(endpoint);
            let params = parse_query_params(query)?;
            validate_query_param_keys(&params, &["addr", "address", "read_timeout_ms"])?;
            let address = parse_required_address_param(&params)?;
            let read_timeout_ms = parse_timeout_param(&params, defaults.prologix_read_timeout_ms)?;
            let (host, port) =
                parse_host_port(endpoint, Some(DEFAULT_PROLOGIX_PORT), "Prologix TCP")?;
            return Ok(Self::PrologixTcp {
                host,
                port,
                address,
                read_timeout_ms,
            });
        }
        if let Some(endpoint) = value.strip_prefix("prologix-serial://") {
            let (path, query) = split_query(endpoint);
            let params = parse_query_params(query)?;
            validate_query_param_keys(
                &params,
                &["addr", "address", "baud_rate", "read_timeout_ms"],
            )?;
            let address = parse_required_address_param(&params)?;
            let read_timeout_ms = parse_timeout_param(&params, defaults.prologix_read_timeout_ms)?;
            let baud_rate =
                parse_optional_u32_param(&params, "baud_rate", DEFAULT_PROLOGIX_BAUD_RATE)?;
            if baud_rate == 0 {
                return Err("Prologix serial baud_rate must be positive".to_string());
            }
            let path = path.trim();
            if path.is_empty() {
                return Err("Prologix serial path must not be empty".to_string());
            }
            return Ok(Self::PrologixSerial {
                path: path.to_string(),
                address,
                baud_rate,
                read_timeout_ms,
            });
        }

        Err(format!("unsupported connection string: {value}"))
    }
}

impl fmt::Display for ConnectionUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { host, port } => write!(formatter, "tcp://{}:{port}", display_host(host)),
            Self::Visa { resource } => write!(formatter, "visa:{resource}"),
            Self::Gpib { board, address } => write!(formatter, "gpib://{board}/{address}"),
            Self::PrologixTcp {
                host,
                port,
                address,
                read_timeout_ms,
            } => write!(
                formatter,
                "prologix-tcp://{}:{port}?addr={address}&read_timeout_ms={read_timeout_ms}",
                display_host(host)
            ),
            Self::PrologixSerial {
                path,
                address,
                baud_rate,
                read_timeout_ms,
            } => write!(
                formatter,
                "prologix-serial://{path}?addr={address}&baud_rate={baud_rate}&read_timeout_ms={read_timeout_ms}"
            ),
        }
    }
}

fn display_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn parse_host_port(
    endpoint: &str,
    default_port: Option<u16>,
    label: &str,
) -> Result<(String, u16), String> {
    let (host, port) = if let Some(rest) = endpoint.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| format!("{label} IPv6 endpoint must be [address]:port or [address]"))?;
        let port = match tail.strip_prefix(':') {
            Some(port) => parse_port(port, label)?,
            None if tail.is_empty() => {
                default_port.ok_or_else(|| format!("{label} endpoint must include a port"))?
            }
            _ => {
                return Err(format!(
                    "{label} IPv6 endpoint must be [address]:port or [address]"
                ));
            }
        };
        (host, port)
    } else if let Some((host, port)) = endpoint.rsplit_once(':') {
        if host.contains(':') {
            return Err(format!(
                "{label} IPv6 endpoint must use [address]:port or [address]"
            ));
        }
        (host, parse_port(port, label)?)
    } else {
        let port = default_port.ok_or_else(|| format!("{label} endpoint must include a port"))?;
        (endpoint, port)
    };
    let host = host.trim();
    if host.is_empty() {
        return Err(format!("{label} host must not be empty"));
    }
    Ok((host.to_string(), port))
}

fn parse_port(value: &str, label: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("invalid {label} port: {value}"))?;
    if port == 0 {
        return Err(format!("{label} port must be in 1..=65535"));
    }
    Ok(port)
}

fn split_query(value: &str) -> (&str, Option<&str>) {
    value
        .split_once('?')
        .map_or((value, None), |(base, query)| (base, Some(query)))
}

fn parse_query_params(query: Option<&str>) -> Result<Vec<(&str, &str)>, String> {
    let Some(query) = query else {
        return Ok(Vec::new());
    };
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once('=')
                .ok_or_else(|| format!("query parameter '{part}' must be key=value"))
        })
        .collect()
}

fn validate_query_param_keys(params: &[(&str, &str)], allowed: &[&str]) -> Result<(), String> {
    for (key, _) in params {
        if !allowed.contains(key) {
            return Err(format!("unsupported query parameter: {key}"));
        }
    }
    Ok(())
}

fn parse_required_address_param(params: &[(&str, &str)]) -> Result<u8, String> {
    let address = find_unique_query_param(params, &["addr", "address"], "Prologix address")?
        .ok_or_else(|| "Prologix connection must include addr=0..30".to_string())?;
    parse_gpib_address(address, "Prologix address")
}

fn parse_timeout_param(params: &[(&str, &str)], default: u64) -> Result<u16, String> {
    let timeout = match find_unique_query_param(params, &["read_timeout_ms"], "read_timeout_ms")? {
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| format!("invalid read_timeout_ms: {value}"))?,
        None => default,
    };
    if !(1..=3000).contains(&timeout) {
        return Err(format!(
            "Prologix read_timeout_ms must be in 1..=3000 (got {timeout})"
        ));
    }
    Ok(timeout as u16)
}

fn parse_optional_u32_param(
    params: &[(&str, &str)],
    key: &str,
    default: u32,
) -> Result<u32, String> {
    let Some(value) = find_unique_query_param(params, &[key], key)? else {
        return Ok(default);
    };
    value
        .parse::<u32>()
        .map_err(|_| format!("invalid {key}: {value}"))
}

fn find_unique_query_param<'a>(
    params: &[(&str, &'a str)],
    keys: &[&str],
    label: &str,
) -> Result<Option<&'a str>, String> {
    let mut values = params
        .iter()
        .filter_map(|(key, value)| keys.contains(key).then_some(*value));
    let value = values.next();
    if values.next().is_some() {
        return Err(format!("{label} must be specified only once"));
    }
    Ok(value)
}

fn parse_gpib_address(value: &str, label: &str) -> Result<u8, String> {
    let address = parse_u8(value, label)?;
    if address > 30 {
        return Err(format!("{label} must be in 0..=30 (got {address})"));
    }
    Ok(address)
}

fn parse_u8(value: &str, label: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .map_err(|_| format!("invalid {label}: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_every_transport() {
        let cases = [
            ("tcp://scope.local:5025", "tcp://scope.local:5025"),
            ("tcp://[2001:db8::1]:5025", "tcp://[2001:db8::1]:5025"),
            ("visa:USB0::1234::INSTR", "visa:USB0::1234::INSTR"),
            ("gpib://0/17", "gpib://0/17"),
            (
                "prologix-tcp://bridge.local?addr=17",
                "prologix-tcp://bridge.local:1234?addr=17&read_timeout_ms=3000",
            ),
            (
                "prologix-serial:///dev/ttyUSB0?address=11",
                "prologix-serial:///dev/ttyUSB0?addr=11&baud_rate=115200&read_timeout_ms=3000",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                ConnectionUri::parse(input, ConnectionDefaults::default())
                    .unwrap()
                    .to_string(),
                expected
            );
        }
    }
}
