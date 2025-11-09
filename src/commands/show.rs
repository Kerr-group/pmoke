use crate::config::Config;
use anyhow::Result;
use lockin_core::RefType;

pub fn show(cfg: &Config) -> Result<()> {
    println!("{:#?}", cfg);

    let cos = RefType::Cos;

    match cos {
        RefType::Cos => println!("Reference type is Cosine"),
        RefType::Sin => println!("Reference type is Sine"),
    }
    Ok(())
}
