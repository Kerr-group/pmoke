use anyhow::Result;
use instruments::rigol::DHO5108;

fn main() -> Result<()> {
    let ip = "10.249.11.19";
    let port = 55255;
    let timeout = None;
    let mut dho = DHO5108::open(ip, port, timeout)?;

    let idn = dho.identify()?;
    println!("IDN: {}", idn);

    let ch1_data = dho.fetch(1, 10_000_000)?;
    println!("CH1 Data Length: {}", ch1_data.len());

    // dho.set_single()?;

    Ok(())
}
