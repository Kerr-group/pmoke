use anyhow::Result;
use instruments::nf::WF1946B;
use instruments::transport::{ScpiConnection, open_scpi_transport};

fn main() -> Result<()> {
    let pad: i32 = 11;
    let transport = open_scpi_transport(&ScpiConnection::Gpib {
        board: 0,
        address: pad,
        timeout_secs: 10,
        use_crlf: false,
    })?;

    let mut wf = WF1946B::new(transport);

    let idn = wf.identify()?;
    println!("*IDN? -> {}", idn);

    Ok(())
}
