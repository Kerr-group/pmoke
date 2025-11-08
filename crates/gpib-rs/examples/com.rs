use gpib_rs::{Instrument, Result, scan_gpib0};

fn main() -> Result<()> {
    match scan_gpib0(5) {
        Ok(found) => println!("{:?}", found),
        Err(e) => eprintln!("Error scanning GPIB: {}", e),
    }

    let inst = Instrument::open(17)?;
    let idn = inst.query_line("*IDN?")?;
    println!("IDN: {}", idn);
    Ok(())
}
