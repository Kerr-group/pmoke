use anyhow::{Context, Result};
use instruments::keithley::Keithley2010;
use instruments::transport::{ScpiConnection, open_scpi_transport};

const HOST: &str = "10.249.11.17";
const PORT: u16 = 1234;
const PAD: u8 = 17;
const READ_TIMEOUT_MS: u16 = 3_000;

fn main() -> Result<()> {
    let connection = ScpiConnection::PrologixTcp {
        host: HOST.to_string(),
        port: PORT,
        address: PAD,
        read_timeout_ms: READ_TIMEOUT_MS,
    };

    eprintln!("Connecting to {connection}");
    let transport = open_scpi_transport(&connection).context("failed to open Prologix TCP")?;
    let mut dmm = Keithley2010::new(transport);

    let idn = dmm.identify().context("failed to query *IDN?")?;
    println!("*IDN? -> {}", idn.trim());

    Ok(())
}
