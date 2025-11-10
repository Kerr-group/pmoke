use crate::config::Config;
use anyhow::Result;

pub fn show(cfg: &Config) -> Result<()> {
    println!("{:#?}", cfg);

    Ok(())
}
