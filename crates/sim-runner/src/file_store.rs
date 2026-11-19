use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const SIDECAR_PREFIX: &str = ".sim-runner-";

#[derive(Debug)]
pub(crate) enum StoreError {
    Io {
        phase: StorePhase,
        path: PathBuf,
        source: io::Error,
    },
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    DurabilityUnconfirmed {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { phase, path, .. } => {
                write!(formatter, "cannot {phase} for {}", path.display())
            }
            Self::TooLarge { path, limit } => {
                write!(
                    formatter,
                    "{} exceeds the {limit} byte limit",
                    path.display()
                )
            }
            Self::DurabilityUnconfirmed { path, .. } => write!(
                formatter,
                "replaced {}, but could not confirm durability by syncing its directory",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::DurabilityUnconfirmed { source, .. } => Some(source),
            Self::TooLarge { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorePhase {
    Read,
    ResolveTarget,
    Lock,
    CreateTemporary,
    Write,
    SyncFile,
    Replace,
}

impl fmt::Display for StorePhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Read => "read input",
            Self::ResolveTarget => "resolve destination",
            Self::Lock => "lock destination",
            Self::CreateTemporary => "create temporary file",
            Self::Write => "write temporary file",
            Self::SyncFile => "sync temporary file",
            Self::Replace => "replace destination",
        })
    }
}

pub(crate) enum Operation<'a> {
    Write(&'a mut dyn Write, &'a [u8]),
    SyncFile(&'a File),
    Replace(NamedTempFile, &'a Path),
    SyncDirectory(&'a Path),
}

impl Operation<'_> {
    pub(crate) fn perform(self) -> io::Result<()> {
        match self {
            Self::Write(file, bytes) => file.write_all(bytes),
            Self::SyncFile(file) => file.sync_all(),
            Self::Replace(temporary, target) => temporary
                .persist(target)
                .map(drop)
                .map_err(|error| error.error),
            Self::SyncDirectory(directory) => File::open(directory)?.sync_all(),
        }
    }
}

pub(crate) struct FileStore;

impl FileStore {
    pub(crate) fn read(&self, path: &Path) -> Result<Vec<u8>, StoreError> {
        let read_error = io_at(StorePhase::Read, path);
        let too_large = || StoreError::TooLarge {
            path: path.to_owned(),
            limit: MAX_BYTES,
        };
        let mut file = File::open(path).map_err(&read_error)?;
        if file.metadata().map_err(&read_error)?.len() > MAX_BYTES {
            return Err(too_large());
        }

        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BYTES)
            .read_to_end(&mut bytes)
            .map_err(&read_error)?;
        // Check growth since opening without extending the output buffer beyond the limit.
        if bytes.len() as u64 == MAX_BYTES {
            let mut extra = [0];
            match file.read_exact(&mut extra) {
                Ok(()) => return Err(too_large()),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
                Err(error) => return Err(read_error(error)),
            }
        }
        Ok(bytes)
    }

    pub(crate) fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        self.write_with(path, bytes, |operation| operation.perform())
    }

    pub(crate) fn write_with(
        &self,
        path: &Path,
        bytes: &[u8],
        mut perform: impl FnMut(Operation<'_>) -> io::Result<()>,
    ) -> Result<(), StoreError> {
        let resolve_error = io_at(StorePhase::ResolveTarget, path);
        let name = path
            .file_name()
            .filter(|name| {
                // Path::file_name strips trailing separators and dot components.
                path.as_os_str()
                    .as_encoded_bytes()
                    .ends_with(name.as_encoded_bytes())
            })
            .ok_or_else(|| {
                resolve_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected a file name",
                ))
            })?;
        if name
            .as_encoded_bytes()
            .starts_with(SIDECAR_PREFIX.as_bytes())
        {
            return Err(resolve_error(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("file names starting with {SIDECAR_PREFIX} are reserved"),
            )));
        }

        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).map_err(resolve_error)?;
        let target = parent.join(name);

        let hash = blake3::hash(name.as_encoded_bytes());
        let lock_path = parent.join(format!("{SIDECAR_PREFIX}{hash}.lock"));
        let lock_error = io_at(StorePhase::Lock, path);
        // Leave the sidecar in place so all waiters keep locking the same inode.
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(&lock_error)?;
        lock.lock().map_err(lock_error)?;

        let mut temporary = tempfile::Builder::new()
            .prefix(SIDECAR_PREFIX)
            .tempfile_in(&parent)
            .map_err(io_at(StorePhase::CreateTemporary, path))?;
        perform(Operation::Write(temporary.as_file_mut(), bytes))
            .map_err(io_at(StorePhase::Write, path))?;
        perform(Operation::SyncFile(temporary.as_file()))
            .map_err(io_at(StorePhase::SyncFile, path))?;
        perform(Operation::Replace(temporary, &target))
            .map_err(io_at(StorePhase::Replace, path))?;
        perform(Operation::SyncDirectory(&parent)).map_err(|source| {
            StoreError::DurabilityUnconfirmed {
                path: path.to_owned(),
                source,
            }
        })
    }
}

fn io_at(phase: StorePhase, path: &Path) -> impl Fn(io::Error) -> StoreError {
    move |source| StoreError::Io {
        phase,
        path: path.to_owned(),
        source,
    }
}
