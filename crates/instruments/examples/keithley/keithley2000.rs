use anyhow::Result;
use instruments::keithley::Keithley2000;
use std::io::{self, Write};

fn main() -> Result<()> {
    if let Ok(found) = gpib_rs::scan_gpib0(5) {
        eprintln!("Found PADs on gpib0: {:?}", found);
    }

    print!("Enter GPIB PAD (0-30): ");
    io::stdout().flush()?; // io::Error -> anyhow::Error

    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    let pad: i32 = s.trim().parse()?; // ParseIntError -> anyhow::Error

    let dmm = Keithley2000::open(pad)?; // InstrumentError -> anyhow::Error

    let idn = dmm.identify()?;
    println!("*IDN? -> {}", idn);

    Ok(())
}
