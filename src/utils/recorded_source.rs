//! Recorded-only noise workflow source adapter (PN-M1, Issue #246).
//!
//! `RecordedSource` is the single validated channel/time abstraction over
//! recorded RAW run directories and recorded CSV files (PN-FR-001). It
//! streams selected original-index blocks with a bounded per-read cap, so a
//! diagnosis never needs a whole-capture CSV export first. Construction
//! performs freeze-before-parse (PN-FR-003): the source tree is hashed into
//! an immutable input view, live inputs are re-verified after every read,
//! and any source change is a hard `source_changed` error, never a silent
//! re-read. Channel and grid bindings are explicit (PN-FR-002): detector,
//! reference, and time identities resolve from the request plus acquisition
//! metadata, and conflicts are rejected without substitution.
//!
//! Recorded-only: no instrument URIs, no acquisition, no network. All paths
//! resolve inside explicit directories; symlinks and canonical-path aliasing
//! are rejected at open time.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use crate::utils::raw_data::{RawTimeAxis, RawVoltageScale};

/// Maximum samples decoded per block read. Bounds per-worker scratch
/// (PN-NFR-003); larger intervals are served as successive bounded reads.
pub const MAX_BLOCK_READ_SAMPLES: usize = 1_048_576;

/// Maximum bytes hashed per file during freeze/view verification. Whole
/// small files hash in one pass; larger captures stream in chunks.
const HASH_CHUNK_BYTES: usize = 8 * 1024 * 1024;

/// Recorded source kind selected explicitly by the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedSourceKind {
    RecordedRaw,
    RecordedCsv,
}

/// Explicit physical channel binding (PN-FR-002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelBinding {
    /// Physical detector channel number (e.g. 2 or 3; never inferred).
    pub detector: u8,
    /// Optional phase-reference channel number, when distinct.
    pub reference: Option<u8>,
    /// Optional witness channel number (descriptive use only).
    pub witness: Option<u8>,
}

/// Explicit grid binding: output stride preserved from the request.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridBinding {
    /// Decimation stride in original samples (must be >= 1).
    pub stride: usize,
}

impl GridBinding {
    fn validate(self) -> Result<()> {
        if self.stride == 0 {
            bail!("noise grid stride must be positive, got 0");
        }
        Ok(())
    }
}

/// Request used to open a recorded source. All identities explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedSourceRequest {
    pub kind: RecordedSourceKind,
    /// Directory holding the recorded RAW run (manifest + channel files),
    /// or the recorded CSV file path, relative to the request directory.
    pub path: String,
    pub channels: ChannelBinding,
    pub grid: GridBinding,
}

/// Immutable frozen view of the exact bytes consumed (PN-FR-003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenInputView {
    pub schema_version: u32,
    /// Canonicalized source directory or file (no symlinks).
    pub canonical_source: String,
    /// Per-file SHA-256 over the exact consumed bytes, keyed by
    /// source-relative portable name.
    pub file_digests: BTreeMap<String, String>,
    /// Manifest/CSV-header bytes digest (metadata identity).
    pub metadata_digest: String,
}

pub const FROZEN_VIEW_SCHEMA_VERSION: u32 = 1;

/// Validated timebase shared by every channel of one source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceTimebase {
    pub axis: RawTimeAxis,
    /// Sample interval in seconds (x_increment).
    pub sample_interval_s: f64,
}

/// One decoded block over original indices `[start, end)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceBlock {
    pub start: u64,
    pub end: u64,
    pub times: Vec<f64>,
    pub detector: Vec<f64>,
    pub reference: Option<Vec<f64>>,
    pub witness: Option<Vec<f64>>,
}

enum SourceBackend {
    Raw {
        dir: PathBuf,
        channels: BTreeMap<u8, RawChannelFile>,
        timebase: SourceTimebase,
    },
    Csv {
        path: PathBuf,
        time_column: Option<usize>,
        detector_column: usize,
        reference_column: Option<usize>,
        witness_column: Option<usize>,
        timebase: SourceTimebase,
    },
}

struct RawChannelFile {
    path: PathBuf,
    scale: RawVoltageScale,
    expected_bytes: usize,
    expected_sha256: Option<String>,
}

impl RawChannelFile {
    /// Declared per-channel checksum (when the manifest carries one) is
    /// enforced at open time; byte-level digests live in the frozen view.
    fn declared_checksum(&self) -> Option<&str> {
        self.expected_sha256.as_deref()
    }
}

/// Validated recorded source with a frozen input view.
pub struct RecordedSource {
    backend: SourceBackend,
    view: FrozenInputView,
    binding: ChannelBinding,
    grid: GridBinding,
    kind: RecordedSourceKind,
}

impl RecordedSource {
    /// Opens, validates, and freezes a recorded source. Rejects instrument
    /// URIs, symlinks, canonical-path aliasing, and channel/grid conflicts.
    pub fn open(request_dir: &Path, request: &RecordedSourceRequest) -> Result<Self> {
        request.grid.validate()?;
        validate_binding(&request.channels)?;
        let joined = join_request_path(request_dir, &request.path)?;
        let canonical = canonical_no_symlink(&joined)?;
        match request.kind {
            RecordedSourceKind::RecordedRaw => Self::open_raw(&canonical, request),
            RecordedSourceKind::RecordedCsv => Self::open_csv(&canonical, request),
        }
    }

    /// Frozen view of the exact consumed bytes (PN-FR-003).
    pub fn frozen_view(&self) -> &FrozenInputView {
        &self.view
    }

    pub fn timebase(&self) -> SourceTimebase {
        match &self.backend {
            SourceBackend::Raw { timebase, .. } => *timebase,
            SourceBackend::Csv { timebase, .. } => *timebase,
        }
    }

    pub fn sample_count(&self) -> usize {
        self.timebase().axis.sample_count
    }

    pub fn kind(&self) -> RecordedSourceKind {
        self.kind
    }

    pub fn binding(&self) -> &ChannelBinding {
        &self.binding
    }

    pub fn grid(&self) -> GridBinding {
        self.grid
    }

    /// Reads original-index samples `[start, end)` with a bounded per-read
    /// cap (PN-FR-001). Verifies live inputs against the frozen view before
    /// and after the read; any change fails with `source_changed`.
    pub fn read_block(&self, start: u64, end: u64) -> Result<SourceBlock> {
        let total = self.sample_count() as u64;
        if end <= start {
            bail!("noise source read needs a non-empty interval [{start}, {end})");
        }
        if end > total {
            bail!("noise source read [{start}, {end}) exceeds recorded length {total}");
        }
        let len = end - start;
        if len > MAX_BLOCK_READ_SAMPLES as u64 {
            bail!(
                "noise source read [{start}, {end}) holds {len} samples above the bounded cap {}",
                MAX_BLOCK_READ_SAMPLES
            );
        }
        self.verify_live()?;
        let block = match &self.backend {
            SourceBackend::Raw { channels, .. } => self.read_raw_block(channels, start, end)?,
            SourceBackend::Csv { .. } => self.read_csv_block(start, end)?,
        };
        self.verify_live()?;
        Ok(block)
    }

    /// Re-verifies live inputs against the frozen view. Any mismatch is a
    /// hard `source_changed` error (PN-FR-003).
    pub fn verify_live(&self) -> Result<()> {
        match &self.backend {
            SourceBackend::Raw { dir, .. } => {
                let current = freeze_raw_dir(dir)?;
                if current != self.view {
                    bail!(
                        "source_changed: recorded RAW source changed after freeze ({})",
                        dir.display()
                    );
                }
                Ok(())
            }
            SourceBackend::Csv { path, .. } => {
                let current = freeze_csv_file(path)?;
                if current.file_digests != self.view.file_digests
                    || current.metadata_digest != self.view.metadata_digest
                {
                    bail!(
                        "source_changed: recorded CSV source changed after freeze ({})",
                        path.display()
                    );
                }
                Ok(())
            }
        }
    }

    fn open_raw(canonical_dir: &Path, request: &RecordedSourceRequest) -> Result<Self> {
        use crate::utils::waveform as waveform_util;
        let view = freeze_raw_dir(canonical_dir)?;
        let loaded = waveform_util::describe_raw_dir_for_noise(canonical_dir)?;
        let mut timebase: Option<SourceTimebase> = None;
        let mut channels = BTreeMap::new();
        for channel in loaded.channels {
            if timebase.is_none() {
                let axis = RawTimeAxis {
                    sample_count: channel.sample_count,
                    x_increment: channel.x_increment,
                    x_origin: channel.x_origin,
                    x_reference: channel.x_reference,
                };
                axis.validate_geometry().map_err(|error| {
                    anyhow::anyhow!("recorded RAW time axis invalid: {error:?}")
                })?;
                timebase = Some(SourceTimebase {
                    axis,
                    sample_interval_s: channel.x_increment,
                });
            }
            let scale = RawVoltageScale {
                y_increment: channel.y_increment,
                y_origin: channel.y_origin,
                y_reference: channel.y_reference,
            };
            scale.validate_geometry().map_err(|error| {
                anyhow::anyhow!("recorded RAW voltage scale invalid: {error:?}")
            })?;
            let path = canonical_dir.join(&channel.file);
            let expected_bytes = channel
                .sample_count
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("recorded RAW sample count overflows"))?;
            channels.insert(
                channel.channel,
                RawChannelFile {
                    path,
                    scale,
                    expected_bytes,
                    expected_sha256: channel.sha256,
                },
            );
        }
        let timebase =
            timebase.ok_or_else(|| anyhow::anyhow!("recorded RAW source holds no channels"))?;
        let detector = request.channels.detector;
        let reference = request.channels.reference;
        let witness = request.channels.witness;
        for channel in std::iter::once(detector).chain(reference).chain(witness) {
            if !channels.contains_key(&channel) {
                bail!(
                    "recorded RAW source has no channel ch{channel} (detector ch{detector} requested)"
                );
            }
        }
        // Cross-channel timebase identity is enforced by the loader: every
        // channel shares sample_count/x_increment/x_origin/x_reference.
        for channel in channels.values() {
            if channel.declared_checksum().is_some() {
                verify_raw_channel_digest(channel)?;
            }
        }
        Ok(Self {
            backend: SourceBackend::Raw {
                dir: canonical_dir.to_path_buf(),
                channels,
                timebase,
            },
            view,
            binding: request.channels.clone(),
            grid: request.grid,
            kind: RecordedSourceKind::RecordedRaw,
        })
    }

    fn open_csv(canonical_path: &Path, request: &RecordedSourceRequest) -> Result<Self> {
        let view = freeze_csv_file(canonical_path)?;
        let (headers, sample_count) = csv_header_and_count(canonical_path)?;
        let time_column = find_time_column(&headers);
        let detector_column =
            find_channel_column(&headers, request.channels.detector).ok_or_else(|| {
                anyhow::anyhow!(
                    "recorded CSV has no ch{} column (headers: {})",
                    request.channels.detector,
                    headers.join(", ")
                )
            })?;
        let reference_column = match request.channels.reference {
            Some(channel) => Some(find_channel_column(&headers, channel).ok_or_else(|| {
                anyhow::anyhow!("recorded CSV has no reference ch{channel} column")
            })?),
            None => None,
        };
        let witness_column = match request.channels.witness {
            Some(channel) => Some(find_channel_column(&headers, channel).ok_or_else(|| {
                anyhow::anyhow!("recorded CSV has no witness ch{channel} column")
            })?),
            None => None,
        };
        if sample_count == 0 {
            bail!(
                "recorded CSV holds no samples: {}",
                canonical_path.display()
            );
        }
        // CSV timebase: explicit time column when present, else uniform
        // index timebase validated on first read by finite/monotonic check.
        let timebase = SourceTimebase {
            axis: RawTimeAxis {
                sample_count,
                x_increment: 1.0,
                x_origin: 0.0,
                x_reference: 0.0,
            },
            sample_interval_s: f64::NAN,
        };
        Ok(Self {
            backend: SourceBackend::Csv {
                path: canonical_path.to_path_buf(),
                time_column,
                detector_column,
                reference_column,
                witness_column,
                timebase,
            },
            view,
            binding: request.channels.clone(),
            grid: request.grid,
            kind: RecordedSourceKind::RecordedCsv,
        })
    }

    fn read_raw_block(
        &self,
        channels: &BTreeMap<u8, RawChannelFile>,
        start: u64,
        end: u64,
    ) -> Result<SourceBlock> {
        let timebase = self.timebase();
        let times: Vec<f64> = (start..end)
            .map(|index| timebase.axis.value_at(index as usize))
            .collect();
        let detector = read_raw_range(
            channels
                .get(&self.binding.detector)
                .ok_or_else(|| anyhow::anyhow!("frozen source lost detector channel"))?,
            start,
            end,
        )?;
        let reference = match self.binding.reference {
            Some(channel) => Some(read_raw_range(
                channels
                    .get(&channel)
                    .ok_or_else(|| anyhow::anyhow!("frozen source lost reference channel"))?,
                start,
                end,
            )?),
            None => None,
        };
        let witness = match self.binding.witness {
            Some(channel) => Some(read_raw_range(
                channels
                    .get(&channel)
                    .ok_or_else(|| anyhow::anyhow!("frozen source lost witness channel"))?,
                start,
                end,
            )?),
            None => None,
        };
        Ok(SourceBlock {
            start,
            end,
            times,
            detector,
            reference,
            witness,
        })
    }

    fn read_csv_block(&self, start: u64, end: u64) -> Result<SourceBlock> {
        let SourceBackend::Csv {
            path,
            time_column,
            detector_column,
            reference_column,
            witness_column,
            ..
        } = &self.backend
        else {
            bail!("noise source backend mismatch reading CSV block");
        };
        let mut wanted = vec![*detector_column];
        if let Some(column) = time_column
            && !wanted.contains(column)
        {
            wanted.push(*column);
        }
        for optional in [reference_column, witness_column] {
            if let Some(column) = optional
                && !wanted.contains(column)
            {
                wanted.push(*column);
            }
        }
        let columns = read_csv_columns(path, &wanted, start as usize, (end - start) as usize)?;
        let column_of = |index: usize| {
            wanted
                .iter()
                .position(|candidate| *candidate == index)
                .map(|position| columns[position].clone())
        };
        let detector = column_of(*detector_column)
            .ok_or_else(|| anyhow::anyhow!("recorded CSV detector column vanished"))?;
        let times = match time_column {
            Some(index) => column_of(*index)
                .ok_or_else(|| anyhow::anyhow!("recorded CSV time column vanished"))?,
            None => (start..end).map(|index| index as f64).collect(),
        };
        for (position, value) in times.iter().enumerate() {
            if !value.is_finite() {
                bail!(
                    "recorded CSV time is non-finite at original sample {}",
                    start + position as u64
                );
            }
            if position > 0 && *value <= times[position - 1] {
                bail!(
                    "recorded CSV time is not strictly increasing at original sample {}",
                    start + position as u64
                );
            }
        }
        let reference = match reference_column {
            Some(index) => Some(
                column_of(*index)
                    .ok_or_else(|| anyhow::anyhow!("recorded CSV reference column vanished"))?,
            ),
            None => None,
        };
        let witness = match witness_column {
            Some(index) => Some(
                column_of(*index)
                    .ok_or_else(|| anyhow::anyhow!("recorded CSV witness column vanished"))?,
            ),
            None => None,
        };
        Ok(SourceBlock {
            start,
            end,
            times,
            detector,
            reference,
            witness,
        })
    }
}

fn validate_binding(binding: &ChannelBinding) -> Result<()> {
    if binding.detector == 0 {
        bail!("noise detector channel must be positive, got 0");
    }
    if let Some(reference) = binding.reference
        && reference == binding.detector
    {
        bail!(
            "noise reference ch{reference} conflicts with detector ch{}; channels must be distinct",
            binding.detector
        );
    }
    if let Some(witness) = binding.witness
        && (witness == binding.detector || Some(witness) == binding.reference)
    {
        bail!("noise witness ch{witness} must be distinct from detector and reference");
    }
    Ok(())
}

fn join_request_path(request_dir: &Path, raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        bail!("noise source path must not be empty");
    }
    for marker in ["..", "://", "\0"] {
        if raw.contains(marker) {
            bail!(
                "noise source path must be an explicit local relative path without '{marker}': {raw}"
            );
        }
    }
    let relative = Path::new(raw);
    if relative.is_absolute() {
        bail!("noise source path must be relative to the request directory: {raw}");
    }
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("noise source path must be a plain relative path: {raw}");
        }
    }
    Ok(request_dir.join(relative))
}

/// Canonicalizes without following through a symlinked source root:
/// intermediate symlinks that resolve back inside the same tree (e.g. the
/// macOS /var -> /private/var firmlink or /tmp symlinks) are resolved, while
/// a final source root that is itself a user-level alias is rejected. The
/// frozen view pins the resolved canonical path, so aliasing callers cannot
/// smuggle two identities for one source (PN-FR-003 aliasing rule).
fn canonical_no_symlink(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("cannot resolve noise source path")?
            .join(path)
    };
    // Resolve `..`/`.` lexically first so traversal is judged on the
    // caller-visible path, not on symlink targets.
    let mut lexical = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                lexical.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !lexical.pop() {
                    bail!("noise source path escapes its root after normalization");
                }
            }
            Component::Normal(part) => {
                lexical.push(part);
            }
        }
    }
    // Walk the lexical path; resolve intermediate symlinks but require the
    // final source root itself to be a real (non-symlink) entry.
    let mut resolved = PathBuf::new();
    let mut components = lexical.components().peekable();
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                resolved.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                resolved.push(part);
                let is_last = components.peek().is_none();
                let metadata = std::fs::symlink_metadata(&resolved).with_context(|| {
                    format!("noise source path not found: {}", resolved.display())
                })?;
                if metadata.file_type().is_symlink() {
                    if is_last {
                        bail!(
                            "noise source path aliases through a symlink (rejected): {}",
                            resolved.display()
                        );
                    }
                    let target = std::fs::read_link(&resolved)
                        .with_context(|| format!("cannot read symlink: {}", resolved.display()))?;
                    let parent = resolved
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("/"));
                    let mut expanded = if target.is_absolute() {
                        target
                    } else {
                        parent.join(target)
                    };
                    for next in components.by_ref() {
                        expanded.push(next.as_os_str());
                    }
                    return canonical_no_symlink(&expanded);
                }
            }
        }
    }
    Ok(resolved)
}

fn sha256_hex_stream(file: &mut File) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(output, "{byte:02x}").expect("hex formatting cannot fail");
    }
    Ok(output)
}

fn hash_regular_file(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("noise source entry not found: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "noise source entry is a symlink (rejected): {}",
            path.display()
        );
    }
    if !metadata.file_type().is_file() {
        bail!(
            "noise source entry must be a regular file: {}",
            path.display()
        );
    }
    let mut file = File::open(path)
        .with_context(|| format!("cannot open noise source file: {}", path.display()))?;
    sha256_hex_stream(&mut file)
}

fn freeze_raw_dir(dir: &Path) -> Result<FrozenInputView> {
    let metadata = std::fs::symlink_metadata(dir)
        .with_context(|| format!("recorded RAW source not found: {}", dir.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "recorded RAW source is a symlink (rejected): {}",
            dir.display()
        );
    }
    if !metadata.file_type().is_dir() {
        bail!("recorded RAW source must be a directory: {}", dir.display());
    }
    let manifest = crate::utils::waveform::raw_manifest_path_for_noise(dir)
        .ok_or_else(|| anyhow::anyhow!("recorded RAW source has no manifest: {}", dir.display()))?;
    let metadata_digest = hash_regular_file(&manifest)?;
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot list recorded RAW source: {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("cannot list recorded RAW source: {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut file_digests = BTreeMap::new();
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("cannot stat source entry: {}", path.display()))?;
        if file_type.is_symlink() {
            bail!(
                "recorded RAW source contains a symlink (rejected): {}",
                path.display()
            );
        }
        if !file_type.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 source entry: {}", path.display()))?
            .to_owned();
        file_digests.insert(name, hash_regular_file(&path)?);
    }
    Ok(FrozenInputView {
        schema_version: FROZEN_VIEW_SCHEMA_VERSION,
        canonical_source: dir.display().to_string(),
        file_digests,
        metadata_digest,
    })
}

fn freeze_csv_file(path: &Path) -> Result<FrozenInputView> {
    let digest = hash_regular_file(path)?;
    let header = csv_header_bytes(path)?;
    let metadata_digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&header);
        let finalized = hasher.finalize();
        let mut output = String::with_capacity(finalized.len() * 2);
        for byte in finalized {
            use std::fmt::Write;
            write!(output, "{byte:02x}").expect("hex formatting cannot fail");
        }
        output
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 CSV name: {}", path.display()))?
        .to_owned();
    Ok(FrozenInputView {
        schema_version: FROZEN_VIEW_SCHEMA_VERSION,
        canonical_source: path.display().to_string(),
        file_digests: BTreeMap::from([(name, digest)]),
        metadata_digest,
    })
}

fn csv_header_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path)
        .with_context(|| format!("cannot open recorded CSV: {}", path.display()))?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let count = file.read(&mut byte)?;
        if count == 0 {
            break;
        }
        header.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
        if header.len() > 1_048_576 {
            bail!("recorded CSV header exceeds 1 MiB: {}", path.display());
        }
    }
    if header.is_empty() {
        bail!("recorded CSV holds no header: {}", path.display());
    }
    Ok(header)
}

fn csv_header_and_count(path: &Path) -> Result<(Vec<String>, usize)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read recorded CSV: {}", path.display()))?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("recorded CSV holds no header: {}", path.display()))?;
    let headers = header
        .split(',')
        .map(|field| field.trim().to_owned())
        .collect();
    Ok((headers, lines.count()))
}

fn find_time_column(headers: &[String]) -> Option<usize> {
    headers.iter().position(|header| {
        matches!(
            header.trim().to_ascii_lowercase().as_str(),
            "time" | "time (s)" | "t" | "t (s)"
        )
    })
}

fn find_channel_column(headers: &[String], channel: u8) -> Option<usize> {
    let wanted = format!("ch{channel}");
    headers.iter().position(|header| {
        let normalized = header.trim().to_ascii_lowercase();
        if let Some(number) = normalized.strip_prefix("ch")
            && let Ok(parsed) = number.parse::<u8>()
        {
            return parsed == channel;
        }
        normalized == wanted
    })
}

fn read_csv_columns(
    path: &Path,
    columns: &[usize],
    start: usize,
    count: usize,
) -> Result<Vec<Vec<f64>>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read recorded CSV: {}", path.display()))?;
    let mut lines = text.lines();
    lines.next();
    let mut selected: Vec<Vec<f64>> = columns.iter().map(|_| Vec::with_capacity(count)).collect();
    for (index, line) in lines.enumerate() {
        if index < start {
            continue;
        }
        if index >= start + count {
            break;
        }
        let fields: Vec<&str> = line.split(',').collect();
        for (position, column) in columns.iter().enumerate() {
            let field = fields
                .get(*column)
                .ok_or_else(|| anyhow::anyhow!("recorded CSV row {index} lacks column {column}"))?;
            let value: f64 = field.trim().parse().with_context(|| {
                format!("recorded CSV row {index} column {column} is not finite f64")
            })?;
            if !value.is_finite() {
                bail!("recorded CSV row {index} column {column} is non-finite");
            }
            selected[position].push(value);
        }
    }
    for (position, values) in selected.iter().enumerate() {
        if values.len() != count {
            bail!(
                "recorded CSV ended early reading column {} (got {}, wanted {count})",
                columns[position],
                values.len()
            );
        }
    }
    Ok(selected)
}

fn verify_raw_channel_digest(spec: &RawChannelFile) -> Result<()> {
    let expected = spec
        .declared_checksum()
        .ok_or_else(|| anyhow::anyhow!("raw channel has no declared checksum"))?;
    let actual = hash_regular_file(&spec.path)?;
    if actual != expected {
        bail!(
            "raw channel checksum mismatch for {}: expected {expected}, got {actual}",
            spec.path.display()
        );
    }
    Ok(())
}

fn read_raw_range(spec: &RawChannelFile, start: u64, end: u64) -> Result<Vec<f64>> {
    let actual = std::fs::symlink_metadata(&spec.path)
        .with_context(|| format!("raw channel file not found: {}", spec.path.display()))?;
    if !actual.file_type().is_file() {
        bail!(
            "raw channel file must be a regular file: {}",
            spec.path.display()
        );
    }
    if actual.len() != spec.expected_bytes as u64 {
        bail!(
            "source_changed: raw channel file size changed: {}",
            spec.path.display()
        );
    }
    if let Some(expected) = spec.declared_checksum() {
        verify_raw_channel_digest(spec).with_context(|| {
            format!(
                "source_changed: raw channel checksum changed: {} (expected {expected})",
                spec.path.display()
            )
        })?;
    }
    let mut file = File::open(&spec.path)
        .with_context(|| format!("cannot open raw channel: {}", spec.path.display()))?;
    file.seek(SeekFrom::Start(start * 2))
        .with_context(|| format!("cannot seek raw channel: {}", spec.path.display()))?;
    let count = (end - start) as usize;
    let mut bytes = vec![0_u8; count * 2];
    file.read_exact(&mut bytes)
        .with_context(|| format!("cannot read raw channel range: {}", spec.path.display()))?;
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| {
            let raw = u16::from_le_bytes([word[0], word[1]]);
            spec.scale.value_at(raw)
        })
        .collect())
}

#[cfg(test)]
#[path = "recorded_source/tests.rs"]
mod tests;
