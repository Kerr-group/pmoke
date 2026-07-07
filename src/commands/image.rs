use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, Connection};
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use instruments::rigol::DhoImageFormat;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{error, fmt};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream};

const IMAGE_DIR: &str = "images";
const DEFAULT_SCOPE_PATH: &str = "C:/screenshot.png";
const DEFAULT_FTP_PATH: &str = "screenshot.png";
const FTP_PORT: u16 = 21;
const FTP_IO_TIMEOUT: Duration = Duration::from_secs(5);
const FTP_TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
const SCOPE_FILE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFormat {
    Png,
    Bmp,
    Jpg,
}

impl ImageFormat {
    fn from_scope_path(path: &str) -> Result<Self> {
        match Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => Ok(Self::Png),
            Some("bmp") => Ok(Self::Bmp),
            Some("jpg") => Ok(Self::Jpg),
            _ => bail!("unsupported screenshot extension in {path}"),
        }
    }

    fn dho(self) -> DhoImageFormat {
        match self {
            Self::Png => DhoImageFormat::Png,
            Self::Bmp => DhoImageFormat::Bmp,
            Self::Jpg => DhoImageFormat::Jpg,
        }
    }

    fn validate_signature(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Bmp => bytes.starts_with(b"BM"),
            Self::Jpg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ImagePlan {
    scope_path: String,
    format: ImageFormat,
    transfer: Option<FtpTransfer>,
}

#[derive(Debug)]
struct FtpTransfer {
    address: SocketAddr,
    remote_path: &'static str,
    temp_path: PathBuf,
    final_path: PathBuf,
}

pub fn image(cfg: &Config) -> Result<()> {
    let plan = prepare_image(cfg)?;
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;
    let saved = capture_image(&mut handler, &plan, false)?;
    report_saved_image(&plan, saved.as_deref());
    Ok(())
}

pub(crate) fn prepare_image(cfg: &Config) -> Result<ImagePlan> {
    let format = ImageFormat::from_scope_path(&cfg.image.scope_path)?;
    let oscilloscope = &cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments.oscilloscope is required"))?
        .oscilloscope;

    let transfer = match &oscilloscope.connection {
        Connection::Tcpip { ip, .. } => {
            if cfg.image.scope_path != DEFAULT_SCOPE_PATH {
                bail!(
                    "TCP screenshot transfer currently supports only {DEFAULT_SCOPE_PATH}; got {}",
                    cfg.image.scope_path
                );
            }
            Some(prepare_ftp_transfer(
                &cfg.source_path,
                &cfg.image.scope_path,
                ip,
            )?)
        }
        Connection::Usbtmc { .. } => None,
        Connection::Gpib { .. } => bail!("DHO5108 image saving does not support GPIB"),
    };

    Ok(ImagePlan {
        scope_path: cfg.image.scope_path.clone(),
        format,
        transfer,
    })
}

pub(crate) fn capture_image(
    handler: &mut OscilloscopeHandler,
    plan: &ImagePlan,
    stop_first: bool,
) -> Result<Option<PathBuf>> {
    if stop_first {
        handler
            .stop()
            .context("failed to stop oscilloscope before screenshot")?;
    }

    let transfer = plan.transfer.as_ref();
    if let Some(transfer) = transfer
        && remove_scope_file_if_present(transfer)
            .context("failed to remove previous oscilloscope Local Disk screenshot")?
    {
        ui::info("removed previous oscilloscope Local Disk screenshot before overwrite");
    }
    let mut save_completed = false;
    let mut local_path = None;
    let result = handler.save_image_with(&plan.scope_path, plan.format.dho(), || {
        save_completed = true;
        local_path = match transfer {
            Some(transfer) => Some(download_ftp(transfer, plan.format)?),
            None => None,
        };
        Ok(local_path.clone())
    });

    match result {
        Ok(saved) => Ok(saved),
        Err(error) if save_completed && transfer.is_some() && is_scope_file_missing(&error) => {
            ui::info("normal image save did not publish a file; using display-data transfer");
            let transfer = transfer.expect("guarded by transfer.is_some()");
            recover_missing_scope_image(handler, plan.format, transfer)
                .map(Some)
                .with_context(|| format!("fallback after normal image save failed: {error}"))
        }
        Err(error) => {
            if save_completed {
                if let Some(path) = local_path.as_deref() {
                    ui::saved(format!("screenshot copy: {}", path.display()));
                }
                ui::warn("screenshot save command completed, but file verification or copy failed");
            }
            Err(error).context("failed to save oscilloscope screenshot")
        }
    }
}

fn recover_missing_scope_image(
    handler: &mut OscilloscopeHandler,
    format: ImageFormat,
    transfer: &FtpTransfer,
) -> Result<PathBuf> {
    let image = handler
        .capture_display_image(format.dho())
        .context("failed to read oscilloscope display image for fallback")?;
    validate_image_bytes(&image, format)
        .context("oscilloscope display image fallback returned invalid data")?;
    upload_ftp(transfer, &image)
        .context("failed to recreate screenshot in oscilloscope Local Disk")?;
    download_ftp(transfer, format).context("failed to copy recreated screenshot from oscilloscope")
}

fn is_scope_file_missing(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
            .and_then(|source| source.downcast_ref::<ScopeFileMissing>())
            .is_some()
    })
}

pub(crate) fn report_saved_image(plan: &ImagePlan, local_path: Option<&Path>) {
    ui::saved(format!("oscilloscope screenshot: {}", plan.scope_path));
    if let Some(path) = local_path {
        ui::saved(format!("screenshot copy: {}", path.display()));
    }
}

fn prepare_ftp_transfer(config_path: &Path, scope_path: &str, ip: &str) -> Result<FtpTransfer> {
    let ip = ip
        .parse::<IpAddr>()
        .with_context(|| format!("invalid oscilloscope TCP/IP address: {ip}"))?;
    let config_parent = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_dir = config_parent.join(IMAGE_DIR);
    ensure_image_directory(&output_dir)?;

    let filename = Path::new(scope_path)
        .file_name()
        .ok_or_else(|| anyhow!("image.scope_path has no filename"))?;
    let final_path = output_dir.join(filename);
    let temp_path = output_dir.join(format!(".{}.tmp", filename.to_string_lossy()));
    ensure_path_absent(&final_path, "screenshot output")?;
    ensure_path_absent(&temp_path, "screenshot temporary output")?;

    Ok(FtpTransfer {
        address: SocketAddr::new(ip, FTP_PORT),
        remote_path: DEFAULT_FTP_PATH,
        temp_path,
        final_path,
    })
}

fn ensure_image_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => bail!(
            "screenshot output directory is not a directory: {}",
            path.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path)
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

fn download_ftp(transfer: &FtpTransfer, format: ImageFormat) -> io::Result<PathBuf> {
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&transfer.temp_path)?;
    let result = download_ftp_inner(transfer, format, output);
    if result.is_err() {
        let _ = fs::remove_file(&transfer.temp_path);
    }
    result
}

fn download_ftp_inner(
    transfer: &FtpTransfer,
    format: ImageFormat,
    mut output: File,
) -> io::Result<PathBuf> {
    let mut ftp = connect_authenticated_ftp(transfer.address)?;
    let remote_path = wait_for_remote_file(&mut ftp, transfer.remote_path, SCOPE_FILE_TIMEOUT)?;
    ui::saved(format!(
        "verified oscilloscope Local Disk file: {remote_path}"
    ));
    let expected_size = query_ftp_size(&mut ftp, &remote_path)?;

    let transfer_deadline = Instant::now()
        .checked_add(FTP_TRANSFER_TIMEOUT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "FTP timeout overflow"))?;
    let copied = ftp
        .retr(&remote_path, |reader| {
            copy_with_deadline(reader, &mut output, transfer_deadline)
                .map_err(FtpError::ConnectionError)
        })
        .map_err(|error| ftp_io_error(&format!("RETR {remote_path}"), error))?;
    if copied == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oscilloscope FTP screenshot is empty",
        ));
    }
    if let Some(expected_size) = expected_size
        && copied != expected_size as u64
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("FTP screenshot size is {copied} bytes, expected {expected_size} bytes"),
        ));
    }
    output.flush()?;
    output.sync_all()?;
    drop(output);
    let _ = ftp.quit();

    validate_image_file(&transfer.temp_path, format)?;
    publish_temp_file(&transfer.temp_path, &transfer.final_path)?;
    Ok(transfer.final_path.clone())
}

fn upload_ftp(transfer: &FtpTransfer, image: &[u8]) -> io::Result<()> {
    let mut ftp = connect_authenticated_ftp(transfer.address)?;
    let deadline = Instant::now()
        .checked_add(FTP_TRANSFER_TIMEOUT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "FTP timeout overflow"))?;
    let mut source = io::Cursor::new(image);
    let mut output = ftp
        .put_with_stream(transfer.remote_path)
        .map_err(|error| ftp_io_error(&format!("STOR {}", transfer.remote_path), error))?;
    let copied = copy_with_deadline(&mut source, &mut output, deadline)?;
    ftp.finalize_put_stream(output)
        .map_err(|error| ftp_io_error(&format!("STOR {}", transfer.remote_path), error))?;
    if copied != image.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!(
                "FTP screenshot upload wrote {copied} bytes, expected {} bytes",
                image.len()
            ),
        ));
    }

    let remote_path = wait_for_remote_file(&mut ftp, transfer.remote_path, SCOPE_FILE_TIMEOUT)?;
    if let Some(size) = query_ftp_size(&mut ftp, &remote_path)?
        && size != image.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "uploaded screenshot is {size} bytes, expected {} bytes",
                image.len()
            ),
        ));
    }
    let _ = ftp.quit();
    ui::saved(format!(
        "recreated oscilloscope Local Disk file: {remote_path}"
    ));
    Ok(())
}

fn remove_scope_file_if_present(transfer: &FtpTransfer) -> io::Result<bool> {
    remove_scope_file_if_present_with_timeout(transfer, SCOPE_FILE_TIMEOUT)
}

fn remove_scope_file_if_present_with_timeout(
    transfer: &FtpTransfer,
    timeout: Duration,
) -> io::Result<bool> {
    let mut ftp = connect_authenticated_ftp(transfer.address)?;
    let entries = ftp
        .nlst(None)
        .map_err(|error| ftp_io_error("NLST", error))?;
    let Some(remote_path) = find_remote_file(&entries, transfer.remote_path) else {
        let _ = ftp.quit();
        return Ok(false);
    };

    ftp.rm(remote_path)
        .map_err(|error| ftp_io_error(&format!("DELE {remote_path}"), error))?;
    wait_for_remote_file_absent(&mut ftp, transfer.remote_path, timeout)?;
    let _ = ftp.quit();
    Ok(true)
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

fn wait_for_remote_file(
    ftp: &mut FtpStream,
    expected_name: &str,
    timeout: Duration,
) -> io::Result<String> {
    let deadline = std::time::Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file timeout overflow"))?;
    let mut last_entries = Vec::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(remote_file_not_found_error(expected_name, &last_entries));
        }
        let entries = ftp
            .nlst(None)
            .map_err(|error| ftp_io_error("NLST", error))?;
        if let Some(path) = find_remote_file(&entries, expected_name) {
            return Ok(path.trim().to_string());
        }
        last_entries = entries;
        let now = Instant::now();
        if now >= deadline {
            return Err(remote_file_not_found_error(expected_name, &last_entries));
        }
        std::thread::sleep(Duration::from_millis(100).min(deadline - now));
    }
}

fn wait_for_remote_file_absent(
    ftp: &mut FtpStream,
    expected_name: &str,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file timeout overflow"))?;
    loop {
        let entries = ftp
            .nlst(None)
            .map_err(|error| ftp_io_error("NLST", error))?;
        if find_remote_file(&entries, expected_name).is_none() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "oscilloscope Local Disk file {expected_name:?} remained visible after deletion"
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(100).min(deadline - now));
    }
}

fn find_remote_file<'a>(entries: &'a [String], expected_name: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|entry| remote_basename(entry).eq_ignore_ascii_case(expected_name))
        .map(String::as_str)
}

fn remote_file_not_found_error(expected_name: &str, entries: &[String]) -> io::Error {
    let listing = if entries.is_empty() {
        "<empty>".to_string()
    } else {
        entries.join(", ")
    };
    io::Error::new(
        io::ErrorKind::NotFound,
        ScopeFileMissing(format!(
            "oscilloscope Local Disk file {expected_name:?} was not found; FTP root contains: {listing}"
        )),
    )
}

#[derive(Debug)]
struct ScopeFileMissing(String);

impl fmt::Display for ScopeFileMissing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl error::Error for ScopeFileMissing {}

fn remote_basename(path: &str) -> &str {
    path.trim().rsplit(['/', '\\']).next().unwrap_or_default()
}

fn query_ftp_size(ftp: &mut FtpStream, remote_path: &str) -> io::Result<Option<usize>> {
    match ftp.size(remote_path) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oscilloscope FTP screenshot is empty",
        )),
        Ok(size) => Ok(Some(size)),
        Err(FtpError::UnexpectedResponse(response))
            if matches!(response.status.code(), 500 | 501 | 502 | 504 | 550) =>
        {
            ui::warn("oscilloscope FTP SIZE is unavailable; validating the downloaded image");
            Ok(None)
        }
        Err(error) => Err(ftp_io_error(&format!("SIZE {remote_path}"), error)),
    }
}

fn connect_ftp(address: SocketAddr, timeout: Duration) -> io::Result<FtpStream> {
    let stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    FtpStream::connect_with_stream(stream).map_err(|error| ftp_io_error("welcome", error))
}

fn connect_authenticated_ftp(address: SocketAddr) -> io::Result<FtpStream> {
    let mut ftp = connect_ftp(address, FTP_IO_TIMEOUT)?.passive_stream_builder(|address| {
        let stream = TcpStream::connect_timeout(&address, FTP_IO_TIMEOUT)
            .map_err(FtpError::ConnectionError)?;
        stream
            .set_read_timeout(Some(FTP_IO_TIMEOUT))
            .map_err(FtpError::ConnectionError)?;
        stream
            .set_write_timeout(Some(FTP_IO_TIMEOUT))
            .map_err(FtpError::ConnectionError)?;
        Ok(stream)
    });
    ftp.login("anonymous", "anonymous@")
        .map_err(|error| ftp_io_error("login", error))?;
    ftp.transfer_type(FileType::Binary)
        .map_err(|error| ftp_io_error("TYPE I", error))?;
    Ok(ftp)
}

fn copy_with_deadline<R: Read + ?Sized, W: Write>(
    reader: &mut R,
    writer: &mut W,
    deadline: Instant,
) -> io::Result<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "oscilloscope FTP screenshot transfer timed out",
            ));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(copied);
        }
        writer.write_all(&buffer[..read])?;
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "FTP byte count overflow"))?;
    }
}

fn validate_image_file(path: &Path, format: ImageFormat) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 8];
    let read = file.read(&mut header)?;
    validate_image_bytes(&header[..read], format)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {}", error, path.display())))
}

fn validate_image_bytes(bytes: &[u8], format: ImageFormat) -> io::Result<()> {
    if !format.validate_signature(bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "screenshot has an invalid image signature",
        ));
    }
    Ok(())
}

fn ftp_io_error(operation: &str, error: FtpError) -> io::Error {
    match error {
        FtpError::ConnectionError(error) => io::Error::new(
            error.kind(),
            format!("oscilloscope FTP {operation} failed: {error}"),
        ),
        error => io::Error::other(format!("oscilloscope FTP {operation} failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn image_formats_are_inferred_and_signatures_checked() {
        assert_eq!(
            ImageFormat::from_scope_path("C:/shot.png").unwrap(),
            ImageFormat::Png
        );
        assert_eq!(
            ImageFormat::from_scope_path("C:/shot.BMP").unwrap(),
            ImageFormat::Bmp
        );
        assert_eq!(
            ImageFormat::from_scope_path("C:/shot.jpg").unwrap(),
            ImageFormat::Jpg
        );
        assert!(ImageFormat::from_scope_path("C:/shot.gif").is_err());
        assert!(ImageFormat::Png.validate_signature(b"\x89PNG\r\n\x1a\n"));
        assert!(ImageFormat::Bmp.validate_signature(b"BMdata"));
        assert!(ImageFormat::Jpg.validate_signature(&[0xff, 0xd8, 0xff, 0xe0]));
    }

    #[test]
    fn remote_basename_accepts_ftp_and_windows_style_paths() {
        assert_eq!(remote_basename("screenshot.png"), "screenshot.png");
        assert_eq!(remote_basename("/screenshot.png"), "screenshot.png");
        assert_eq!(remote_basename("C:/screenshot.png"), "screenshot.png");
        assert_eq!(remote_basename(r"C:\screenshot.png"), "screenshot.png");
    }

    #[test]
    fn image_directory_rejects_files_and_symbolic_links() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let file = dir.join("images");
        fs::write(&file, b"not a directory").unwrap();
        assert!(ensure_image_directory(&file).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_transfer_uses_config_sibling_directory_and_refuses_existing_outputs() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let config_path = dir.join("config.toml");

        let transfer = prepare_ftp_transfer(&config_path, DEFAULT_SCOPE_PATH, "127.0.0.1").unwrap();
        assert_eq!(transfer.final_path, dir.join("images/screenshot.png"));
        assert_eq!(transfer.temp_path, dir.join("images/.screenshot.png.tmp"));
        assert!(dir.join("images").is_dir());

        fs::write(&transfer.final_path, b"existing").unwrap();
        assert!(prepare_ftp_transfer(&config_path, DEFAULT_SCOPE_PATH, "127.0.0.1").is_err());
        fs::remove_file(&transfer.final_path).unwrap();
        fs::write(&transfer.temp_path, b"partial").unwrap();
        assert!(prepare_ftp_transfer(&config_path, DEFAULT_SCOPE_PATH, "127.0.0.1").is_err());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn image_validation_rejects_mismatched_signature() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let path = dir.join("screenshot.png");
        fs::write(&path, b"not png").unwrap();
        assert!(validate_image_file(&path, ImageFormat::Png).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_download_uses_anonymous_binary_passive_transfer_and_no_clobber_finalize() {
        let image = b"\x89PNG\r\n\x1a\npayload".to_vec();
        let (address, server) = spawn_ftp_server(image.clone());
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let transfer = FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: dir.join(".screenshot.png.tmp"),
            final_path: dir.join("screenshot.png"),
        };

        let path = download_ftp(&transfer, ImageFormat::Png).unwrap();

        assert_eq!(path, transfer.final_path);
        assert_eq!(fs::read(&path).unwrap(), image);
        assert!(!transfer.temp_path.exists());
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_download_succeeds_when_server_does_not_support_size() {
        let image = b"\x89PNG\r\n\x1a\npayload".to_vec();
        let (address, server) = spawn_ftp_server_with_size_support(image.clone(), false);
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let transfer = FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: dir.join(".screenshot.png.tmp"),
            final_path: dir.join("screenshot.png"),
        };

        let path = download_ftp(&transfer, ImageFormat::Png).unwrap();

        assert_eq!(fs::read(path).unwrap(), image);
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_download_waits_until_scope_file_appears() {
        let image = b"\x89PNG\r\n\x1a\npayload".to_vec();
        let (address, server) = spawn_ftp_server_with_options(image.clone(), true, 1);
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let transfer = FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: dir.join(".screenshot.png.tmp"),
            final_path: dir.join("screenshot.png"),
        };

        let path = download_ftp(&transfer, ImageFormat::Png).unwrap();

        assert_eq!(fs::read(path).unwrap(), image);
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_file_wait_stops_at_deadline_when_scope_file_never_appears() {
        let (address, server) = spawn_ftp_server_with_options(b"unused".to_vec(), true, usize::MAX);
        let mut ftp = connect_ftp(address, Duration::from_secs(2)).unwrap();
        ftp.login("anonymous", "anonymous@").unwrap();
        ftp.transfer_type(FileType::Binary).unwrap();
        let started = Instant::now();

        let error = wait_for_remote_file(&mut ftp, DEFAULT_FTP_PATH, Duration::from_millis(25))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < Duration::from_millis(500));
        ftp.quit().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn ftp_upload_recreates_missing_scope_file() {
        let image = b"\x89PNG\r\n\x1a\npayload".to_vec();
        let (address, server) = spawn_ftp_upload_server(image.clone());
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let transfer = FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: dir.join(".screenshot.png.tmp"),
            final_path: dir.join("screenshot.png"),
        };

        upload_ftp(&transfer, &image).unwrap();

        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scope_overwrite_removes_existing_file_and_verifies_absence() {
        let (address, server) = spawn_ftp_delete_server(true, true);
        let transfer = test_ftp_transfer(address);

        assert!(
            remove_scope_file_if_present_with_timeout(&transfer, Duration::from_secs(1)).unwrap()
        );

        server.join().unwrap();
    }

    #[test]
    fn scope_overwrite_does_nothing_when_file_is_already_absent() {
        let (address, server) = spawn_ftp_delete_server(false, true);
        let transfer = test_ftp_transfer(address);

        assert!(
            !remove_scope_file_if_present_with_timeout(&transfer, Duration::from_secs(1)).unwrap()
        );

        server.join().unwrap();
    }

    #[test]
    fn scope_overwrite_fails_if_deleted_file_remains_visible() {
        let (address, server) = spawn_ftp_delete_server(true, false);
        let transfer = test_ftp_transfer(address);

        let error = remove_scope_file_if_present_with_timeout(&transfer, Duration::from_millis(25))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        server.join().unwrap();
    }

    #[test]
    fn fallback_detection_only_accepts_scope_file_missing_error() {
        let scope_missing = anyhow::Error::new(remote_file_not_found_error(
            DEFAULT_FTP_PATH,
            &["existing.png".to_string()],
        ))
        .context("FTP verification failed");
        let local_missing = anyhow::Error::new(io::Error::new(
            io::ErrorKind::NotFound,
            "PC output directory missing",
        ));

        assert!(is_scope_file_missing(&scope_missing));
        assert!(!is_scope_file_missing(&local_missing));
    }

    #[test]
    fn ftp_copy_rejects_an_expired_transfer_deadline() {
        let mut reader = io::Cursor::new(b"payload");
        let mut output = Vec::new();

        let error = copy_with_deadline(&mut reader, &mut output, Instant::now()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(output.is_empty());
    }

    #[test]
    fn screenshot_publish_rolls_back_final_file_when_temp_cleanup_fails() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let temp_path = dir.join(".screenshot.png.tmp");
        let final_path = dir.join("screenshot.png");
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

    #[test]
    fn ftp_download_removes_temporary_file_after_signature_error() {
        let (address, server) = spawn_ftp_server(b"not a png".to_vec());
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let transfer = FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: dir.join(".screenshot.png.tmp"),
            final_path: dir.join("screenshot.png"),
        };

        let error = download_ftp(&transfer, ImageFormat::Png).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!transfer.temp_path.exists());
        assert!(!transfer.final_path.exists());
        server.join().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ftp_connect_times_out_while_waiting_for_welcome_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(250));
        });

        let error = match connect_ftp(address, Duration::from_millis(25)) {
            Ok(_) => panic!("FTP welcome response unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));

        server.join().unwrap();
    }

    #[test]
    fn ftp_download_does_not_remove_temporary_file_owned_by_another_writer() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let temp_path = dir.join(".screenshot.png.tmp");
        fs::write(&temp_path, b"other writer").unwrap();
        let transfer = FtpTransfer {
            address: "127.0.0.1:1".parse().unwrap(),
            remote_path: DEFAULT_FTP_PATH,
            temp_path: temp_path.clone(),
            final_path: dir.join("screenshot.png"),
        };

        assert!(download_ftp(&transfer, ImageFormat::Png).is_err());
        assert_eq!(fs::read(&temp_path).unwrap(), b"other writer");

        fs::remove_dir_all(dir).unwrap();
    }

    fn spawn_ftp_server(image: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<()>) {
        spawn_ftp_server_with_size_support(image, true)
    }

    fn spawn_ftp_server_with_size_support(
        image: Vec<u8>,
        size_supported: bool,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        spawn_ftp_server_with_options(image, size_supported, 0)
    }

    fn spawn_ftp_server_with_options(
        image: Vec<u8>,
        size_supported: bool,
        missing_nlst_responses: usize,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let control_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = control_listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = control_listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut control = BufReader::new(stream);
            writeln!(control.get_mut(), "220 DHO FTP ready").unwrap();
            control.get_mut().flush().unwrap();
            let mut data_listener = None;
            let mut nlst_count = 0;

            loop {
                let mut command = String::new();
                control.read_line(&mut command).unwrap();
                let command = command.trim_end();
                match command {
                    "USER anonymous" => write_ftp_response(&mut control, "331 Password required"),
                    "PASS anonymous@" => write_ftp_response(&mut control, "230 Logged in"),
                    "TYPE I" => write_ftp_response(&mut control, "200 Binary mode"),
                    "SIZE screenshot.png" => {
                        if size_supported {
                            write_ftp_response(&mut control, &format!("213 {}", image.len()));
                        } else {
                            write_ftp_response(&mut control, "550 Error");
                        }
                    }
                    "PASV" => {
                        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                        let port = listener.local_addr().unwrap().port();
                        write_ftp_response(
                            &mut control,
                            &format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})",
                                port / 256,
                                port % 256
                            ),
                        );
                        data_listener = Some(listener);
                    }
                    "NLST" => {
                        write_ftp_response(&mut control, "150 Opening data connection");
                        let (mut data, _) = data_listener.take().unwrap().accept().unwrap();
                        writeln!(data, "existing.png\r").unwrap();
                        if nlst_count >= missing_nlst_responses {
                            writeln!(data, "screenshot.png\r").unwrap();
                        }
                        nlst_count += 1;
                        data.flush().unwrap();
                        drop(data);
                        write_ftp_response(&mut control, "226 Transfer complete");
                    }
                    "RETR screenshot.png" => {
                        write_ftp_response(&mut control, "150 Opening data connection");
                        let (mut data, _) = data_listener.take().unwrap().accept().unwrap();
                        data.write_all(&image).unwrap();
                        data.flush().unwrap();
                        drop(data);
                        write_ftp_response(&mut control, "226 Transfer complete");
                    }
                    "QUIT" => {
                        write_ftp_response(&mut control, "221 Goodbye");
                        break;
                    }
                    other => panic!("unexpected FTP command: {other:?}"),
                }
            }
        });
        (address, handle)
    }

    fn spawn_ftp_upload_server(
        expected_image: Vec<u8>,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let control_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = control_listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = control_listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut control = BufReader::new(stream);
            write_ftp_response(&mut control, "220 DHO FTP ready");
            let mut data_listener = None;
            let mut uploaded = Vec::new();

            loop {
                let mut command = String::new();
                control.read_line(&mut command).unwrap();
                match command.trim_end() {
                    "USER anonymous" => write_ftp_response(&mut control, "331 Password required"),
                    "PASS anonymous@" => write_ftp_response(&mut control, "230 Logged in"),
                    "TYPE I" => write_ftp_response(&mut control, "200 Binary mode"),
                    "PASV" => {
                        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                        let port = listener.local_addr().unwrap().port();
                        write_ftp_response(
                            &mut control,
                            &format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})",
                                port / 256,
                                port % 256
                            ),
                        );
                        data_listener = Some(listener);
                    }
                    "STOR screenshot.png" => {
                        write_ftp_response(&mut control, "150 Opening data connection");
                        let (mut data, _) = data_listener.take().unwrap().accept().unwrap();
                        data.read_to_end(&mut uploaded).unwrap();
                        assert_eq!(uploaded, expected_image);
                        write_ftp_response(&mut control, "226 Transfer complete");
                    }
                    "NLST" => {
                        write_ftp_response(&mut control, "150 Opening data connection");
                        let (mut data, _) = data_listener.take().unwrap().accept().unwrap();
                        writeln!(data, "screenshot.png\r").unwrap();
                        data.flush().unwrap();
                        drop(data);
                        write_ftp_response(&mut control, "226 Transfer complete");
                    }
                    "SIZE screenshot.png" => {
                        write_ftp_response(&mut control, &format!("213 {}", expected_image.len()))
                    }
                    "QUIT" => {
                        write_ftp_response(&mut control, "221 Goodbye");
                        break;
                    }
                    other => panic!("unexpected FTP command: {other:?}"),
                }
            }
        });
        (address, handle)
    }

    fn spawn_ftp_delete_server(
        initially_present: bool,
        deletion_effective: bool,
    ) -> (SocketAddr, std::thread::JoinHandle<()>) {
        let control_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = control_listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = control_listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut control = BufReader::new(stream);
            write_ftp_response(&mut control, "220 DHO FTP ready");
            let mut data_listener = None;
            let mut present = initially_present;

            loop {
                let mut command = String::new();
                if control.read_line(&mut command).unwrap() == 0 {
                    break;
                }
                match command.trim_end() {
                    "USER anonymous" => write_ftp_response(&mut control, "331 Password required"),
                    "PASS anonymous@" => write_ftp_response(&mut control, "230 Logged in"),
                    "TYPE I" => write_ftp_response(&mut control, "200 Binary mode"),
                    "PASV" => {
                        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
                        let port = listener.local_addr().unwrap().port();
                        write_ftp_response(
                            &mut control,
                            &format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})",
                                port / 256,
                                port % 256
                            ),
                        );
                        data_listener = Some(listener);
                    }
                    "NLST" => {
                        write_ftp_response(&mut control, "150 Opening data connection");
                        let (mut data, _) = data_listener.take().unwrap().accept().unwrap();
                        writeln!(data, "existing.png\r").unwrap();
                        if present {
                            writeln!(data, "screenshot.png\r").unwrap();
                        }
                        data.flush().unwrap();
                        drop(data);
                        write_ftp_response(&mut control, "226 Transfer complete");
                    }
                    "DELE screenshot.png" => {
                        if deletion_effective {
                            present = false;
                        }
                        write_ftp_response(&mut control, "250 File deleted");
                    }
                    "QUIT" => {
                        write_ftp_response(&mut control, "221 Goodbye");
                        break;
                    }
                    other => panic!("unexpected FTP command: {other:?}"),
                }
            }
        });
        (address, handle)
    }

    fn test_ftp_transfer(address: SocketAddr) -> FtpTransfer {
        FtpTransfer {
            address,
            remote_path: DEFAULT_FTP_PATH,
            temp_path: PathBuf::from("unused.tmp"),
            final_path: PathBuf::from("unused.png"),
        }
    }

    fn write_ftp_response(reader: &mut BufReader<TcpStream>, response: &str) {
        writeln!(reader.get_mut(), "{response}").unwrap();
        reader.get_mut().flush().unwrap();
    }

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pmoke_image_test_{}_{}_{}",
            std::process::id(),
            nanos,
            sequence
        ))
    }
}
