use anyhow::Result;
use instruments::nf::WF1946B;

fn main() -> Result<()> {
    let pad: i32 = 11;

    let wf = WF1946B::open(pad)?;

    let idn = wf.identify()?;
    println!("*IDN? -> {}", idn);

    Ok(())
}
