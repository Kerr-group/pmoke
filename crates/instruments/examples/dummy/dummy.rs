use instruments::dummy::DummyInstrument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dmm = DummyInstrument::open(5)?;

    let idn = dmm.query_line("*IDN?")?;
    println!("*IDN? -> {}", idn);

    Ok(())
}
