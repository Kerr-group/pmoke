use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, Connection};
use crate::ui;
use anyhow::{Context, Result, anyhow, bail};
use instruments::rigol::DhoImageFormat;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream};

const IMAGE_DIR: &str = "images";
const DEFAULT_SCOPE_PATH: &str = "C:/screenshot.png";
const DEFAULT_FTP_PATH: &str = "/screenshot.png";
const FTP_PORT: u16 = 21;
const FTP_TIMEOUT: Duration = Duration::from_secs(30);

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
            Some(prepare_ftp_transfer(cfg, ip)?)
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
        Err(error) => {
            if save_completed {
                report_saved_image(plan, local_path.as_deref());
                ui::warn("screenshot was saved, but finalization failed");
            }
            Err(error).context("failed to save oscilloscope screenshot")
        }
    }
}

pub(crate) fn report_saved_image(plan: &ImagePlan, local_path: Option<&Path>) {
    ui::saved(format!("oscilloscope screenshot: {}", plan.scope_path));
    if let Some(path) = local_path {
        ui::saved(format!("screenshot copy: {}", path.display()));
    }
}

fn prepare_ftp_transfer(cfg: &Config, ip: &str) -> Result<FtpTransfer> {
    let ip = ip
        .parse::<IpAddr>()
        .with_context(|| format!("invalid oscilloscope TCP/IP address: {ip}"))?;
    let config_parent = cfg
        .source_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_dir = config_parent.join(IMAGE_DIR);
    ensure_image_directory(&output_dir)?;

    let filename = Path::new(&cfg.image.scope_path)
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
    let result = download_ftp_inner(transfer, format);
    if result.is_err() {
        let _ = fs::remove_file(&transfer.temp_path);
    }
    result
}

fn download_ftp_inner(transfer: &FtpTransfer, format: ImageFormat) -> io::Result<PathBuf> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&transfer.temp_path)?;

    let mut ftp = FtpStream::connect_timeout(transfer.address, FTP_TIMEOUT)
        .map_err(ftp_io_error)?
        .passive_stream_builder(|address| {
            let stream = TcpStream::connect_timeout(&address, FTP_TIMEOUT)
                .map_err(FtpError::ConnectionError)?;
            stream
                .set_read_timeout(Some(FTP_TIMEOUT))
                .map_err(FtpError::ConnectionError)?;
            stream
                .set_write_timeout(Some(FTP_TIMEOUT))
                .map_err(FtpError::ConnectionError)?;
            Ok(stream)
        });
    ftp.get_ref().set_read_timeout(Some(FTP_TIMEOUT))?;
    ftp.get_ref().set_write_timeout(Some(FTP_TIMEOUT))?;
    ftp.login("anonymous", "anonymous@").map_err(ftp_io_error)?;
    ftp.transfer_type(FileType::Binary).map_err(ftp_io_error)?;
    let expected_size = ftp.size(transfer.remote_path).map_err(ftp_io_error)?;
    if expected_size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oscilloscope FTP screenshot is empty",
        ));
    }

    let copied = ftp
        .retr(transfer.remote_path, |reader| {
            io::copy(reader, &mut output).map_err(FtpError::ConnectionError)
        })
        .map_err(ftp_io_error)?;
    if copied != expected_size as u64 {
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
    fs::hard_link(&transfer.temp_path, &transfer.final_path)?;
    fs::remove_file(&transfer.temp_path)?;
    Ok(transfer.final_path.clone())
}

fn validate_image_file(path: &Path, format: ImageFormat) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 8];
    let read = file.read(&mut header)?;
    if !format.validate_signature(&header[..read]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "FTP screenshot has an invalid image signature: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn ftp_io_error(error: FtpError) -> io::Error {
    io::Error::other(format!("oscilloscope FTP error: {error}"))
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
    fn image_directory_rejects_files_and_symbolic_links() {
        let dir = unique_test_dir();
        fs::create_dir(&dir).unwrap();
        let file = dir.join("images");
        fs::write(&file, b"not a directory").unwrap();
        assert!(ensure_image_directory(&file).is_err());
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

    fn spawn_ftp_server(image: Vec<u8>) -> (SocketAddr, std::thread::JoinHandle<()>) {
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

            loop {
                let mut command = String::new();
                control.read_line(&mut command).unwrap();
                let command = command.trim_end();
                match command {
                    "USER anonymous" => write_ftp_response(&mut control, "331 Password required"),
                    "PASS anonymous@" => write_ftp_response(&mut control, "230 Logged in"),
                    "TYPE I" => write_ftp_response(&mut control, "200 Binary mode"),
                    "SIZE /screenshot.png" => {
                        write_ftp_response(&mut control, &format!("213 {}", image.len()));
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
                    "RETR /screenshot.png" => {
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
