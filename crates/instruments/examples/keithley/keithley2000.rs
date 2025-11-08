use instruments::keithley::Keithley2000;
use std::io::{self, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(found) = gpib_rs::scan_gpib0(5) {
        eprintln!("Found PADs on gpib0: {:?}", found);
    }
    print!("Enter GPIB PAD (0-30): ");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    let pad: i32 = s.trim().parse()?;

    let dmm = Keithley2000::open(pad)?;

    let idn = dmm.idn()?;
    println!("*IDN? -> {}", idn);

    Ok(())
}
