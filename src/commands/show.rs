use crate::config::Config;
use lockin_core::RefType;

pub fn run(cfg: &Config) {
    println!("{:#?}", cfg);

    let cos = RefType::Cos;

    match cos {
        RefType::Cos => println!("Reference type is Cosine"),
        RefType::Sin => println!("Reference type is Sine"),
    }
}
