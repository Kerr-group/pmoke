use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const SCREENSHOT_FILENAME: &str = "oscilloscope.png";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug)]
pub(crate) struct ScreenshotPlan {
    temp_path: PathBuf,
    final_path: PathBuf,
}

pub fn screenshot(cfg: &Config) -> Result<()> {
    let plan = prepare_screenshot(cfg)?;
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;
    let saved = capture_screenshot(&mut handler, &plan, false)?;
    report_saved_screenshot(&saved);
    Ok(())
}

pub(crate) fn prepare_screenshot(cfg: &Config) -> Result<ScreenshotPlan> {
    cfg.instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments.oscilloscope is required"))?;
    prepare_screenshot_path(&cfg.paths().oscilloscope_screenshot())
}

pub(crate) fn prepare_screenshot_path(final_path: &Path) -> Result<ScreenshotPlan> {
    let output_dir = final_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_image_directory(output_dir)?;
    let temp_path = output_dir.join(format!(".{SCREENSHOT_FILENAME}.tmp"));
    ensure_path_absent(final_path, "screenshot output")?;
    ensure_path_absent(&temp_path, "screenshot temporary output")?;

    Ok(ScreenshotPlan {
        temp_path,
        final_path: final_path.to_path_buf(),
    })
}

pub(crate) fn capture_screenshot(
    handler: &mut OscilloscopeHandler,
    plan: &ScreenshotPlan,
    stop_first: bool,
) -> Result<PathBuf> {
    if stop_first {
        handler
            .stop()
            .context("failed to stop oscilloscope before screenshot")?;
    }

    let image = handler
        .capture_display_png()
        .context("failed to read oscilloscope display image")?;
    write_display_image(plan, &image).context("failed to save oscilloscope screenshot to PC")
}

pub(crate) fn report_saved_screenshot(path: &Path) {
    ui::saved(format!("oscilloscope screenshot: {}", path.display()));
}

fn write_display_image(plan: &ScreenshotPlan, image: &[u8]) -> io::Result<PathBuf> {
    validate_png_bytes(image)?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&plan.temp_path)?;
    let result = write_display_image_inner(plan, image, output);
    if result.is_err() {
        let _ = fs::remove_file(&plan.temp_path);
    }
    result
}

fn write_display_image_inner(
    plan: &ScreenshotPlan,
    image: &[u8],
    mut output: File,
) -> io::Result<PathBuf> {
    output.write_all(image)?;
    output.flush()?;
    output.sync_all()?;
    drop(output);

    validate_png_file(&plan.temp_path)?;
    publish_temp_file(&plan.temp_path, &plan.final_path)?;
    Ok(plan.final_path.clone())
}

fn ensure_image_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => bail!(
            "screenshot output directory is not a directory: {}",
            path.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)
            .with_context(|| format!("failed to create screenshot directory: {}", path.display())),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect screenshot directory: {}", path.display())),
    }
}

fn ensure_path_absent(path: &Path, description: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("{description} already exists: {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn publish_temp_file(temp_path: &Path, final_path: &Path) -> io::Result<()> {
    publish_temp_file_with(temp_path, final_path, |path| fs::remove_file(path))
}

fn publish_temp_file_with<F>(temp_path: &Path, final_path: &Path, remove_temp: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    fs::hard_link(temp_path, final_path)?;
    if let Err(cleanup_error) = remove_temp(temp_path) {
        return match fs::remove_file(final_path) {
            Ok(()) => Err(cleanup_error),
            Err(rollback_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!(
                    "failed to remove screenshot temporary file: {cleanup_error}; additionally failed to roll back {}: {rollback_error}",
                    final_path.display()
                ),
            )),
        };
    }
    Ok(())
}

fn validate_png_file(path: &Path) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; PNG_SIGNATURE.len()];
    let read = file.read(&mut header)?;
    validate_png_bytes(&header[..read])
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {}", error, path.display())))
}

fn validate_png_bytes(bytes: &[u8]) -> io::Result<()> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "screenshot has an invalid PNG signature",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn screenshot_output_uses_config_sibling_directory_and_refuses_existing_outputs() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let output = dir.join("acquisition/screenshots/oscilloscope.png");

        let plan = prepare_screenshot_path(&output).unwrap();
        assert_eq!(plan.final_path, output);
        assert_eq!(
            plan.temp_path,
            dir.join("acquisition/screenshots/.oscilloscope.png.tmp")
        );
        assert!(dir.join("acquisition/screenshots").is_dir());

        fs::write(&plan.final_path, b"existing").unwrap();
        assert!(prepare_screenshot_path(&output).is_err());
        fs::remove_file(&plan.final_path).unwrap();
        fs::write(&plan.temp_path, b"partial").unwrap();
        assert!(prepare_screenshot_path(&output).is_err());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn screenshot_directory_rejects_non_directories() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let file = dir.join("screenshot");
        fs::write(&file, b"not a directory").unwrap();

        assert!(ensure_image_directory(&file).is_err());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn display_image_is_validated_and_published_atomically() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let plan = test_screenshot_plan(&dir);
        let image = b"\x89PNG\r\n\x1a\npayload";

        let path = write_display_image(&plan, image).unwrap();

        assert_eq!(path, plan.final_path);
        assert_eq!(fs::read(path).unwrap(), image);
        assert!(!plan.temp_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_display_image_does_not_create_output() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let plan = test_screenshot_plan(&dir);

        let error = write_display_image(&plan, b"not a png").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!plan.temp_path.exists());
        assert!(!plan.final_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn display_image_does_not_remove_temporary_file_owned_by_another_writer() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let plan = test_screenshot_plan(&dir);
        fs::write(&plan.temp_path, b"other writer").unwrap();

        assert!(write_display_image(&plan, PNG_SIGNATURE).is_err());
        assert_eq!(fs::read(&plan.temp_path).unwrap(), b"other writer");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn late_output_collision_preserves_existing_file_and_removes_temporary_file() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let plan = test_screenshot_plan(&dir);
        fs::write(&plan.final_path, b"other screenshot").unwrap();

        let error = write_display_image(&plan, b"\x89PNG\r\n\x1a\npayload").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&plan.final_path).unwrap(), b"other screenshot");
        assert!(!plan.temp_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn screenshot_publish_rolls_back_final_file_when_temp_cleanup_fails() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let temp_path = dir.join(".oscilloscope.png.tmp");
        let final_path = dir.join("oscilloscope.png");
        fs::write(&temp_path, b"complete image").unwrap();

        let error = publish_temp_file_with(&temp_path, &final_path, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected cleanup failure",
            ))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(temp_path.exists());
        assert!(!final_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    fn test_screenshot_plan(dir: &Path) -> ScreenshotPlan {
        ScreenshotPlan {
            temp_path: dir.join(".oscilloscope.png.tmp"),
            final_path: dir.join("oscilloscope.png"),
        }
    }

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pmoke_screenshot_test_{}_{}_{}",
            std::process::id(),
            nanos,
            sequence
        ))
    }
}
