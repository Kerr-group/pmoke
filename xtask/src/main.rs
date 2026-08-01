use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

mod config_schema;
mod markdown;

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next()) {
        (Some("docs-export"), None) => export_docs(&workspace_root()),
        _ => bail!("usage: cargo xtask docs-export"),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be inside the workspace")
        .to_path_buf()
}

fn export_docs(root: &Path) -> Result<()> {
    let generated = root.join("website/generated");
    fs::create_dir_all(&generated)
        .with_context(|| format!("failed to create {}", generated.display()))?;

    let mut cli_json = serde_json::to_string_pretty(&pmoke::docs::cli_reference())?;
    cli_json.push('\n');
    write_if_changed(&generated.join("cli-reference.json"), cli_json.as_bytes())?;

    let config_reference = pmoke::config::config_reference();
    let mut config_json = serde_json::to_string_pretty(&config_reference)?;
    config_json.push('\n');
    write_if_changed(
        &generated.join("config-reference.json"),
        config_json.as_bytes(),
    )?;

    let mut schema_json = serde_json::to_string_pretty(&config_schema::build(&config_reference))?;
    schema_json.push('\n');
    write_if_changed(
        &root.join("website/public/config.schema.json"),
        schema_json.as_bytes(),
    )?;

    for locale in [markdown::Locale::English, markdown::Locale::Japanese] {
        write_if_changed(
            &root.join(format!(
                "website/content/docs/{}/cli/reference.mdx",
                locale.code()
            )),
            markdown::render_cli(&pmoke::docs::cli_reference(), locale)?.as_bytes(),
        )?;
        write_if_changed(
            &root.join(format!(
                "website/content/docs/{}/configuration/reference.mdx",
                locale.code()
            )),
            markdown::render_config(&config_reference, locale).as_bytes(),
        )?;
    }
    Ok(())
}

fn write_if_changed(path: &Path, content: &[u8]) -> Result<()> {
    if fs::read(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}
