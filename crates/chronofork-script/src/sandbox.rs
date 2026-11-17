//! Relative-path sandbox for script file builtins.
//!
//! This is a deterministic capability boundary for experiments, not an OS
//! security boundary against hostile native code. Scripts cannot name an
//! absolute path, leave the configured root through `.`/`..`, follow a symlink
//! out of the root, or read a directory. Failure values never echo a host path.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Kind of sandbox failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SandboxErrorKind {
    Root,
    Path,
    Io,
    TooLarge,
}

/// A typed sandbox construction or file-access failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxError {
    kind: SandboxErrorKind,
}

impl SandboxError {
    pub(crate) const fn root() -> Self {
        Self {
            kind: SandboxErrorKind::Root,
        }
    }

    pub(crate) const fn path() -> Self {
        Self {
            kind: SandboxErrorKind::Path,
        }
    }

    pub(crate) const fn io() -> Self {
        Self {
            kind: SandboxErrorKind::Io,
        }
    }

    pub(crate) const fn too_large() -> Self {
        Self {
            kind: SandboxErrorKind::TooLarge,
        }
    }

    pub(crate) const fn kind(self) -> SandboxErrorKind {
        self.kind
    }

    /// Returns the stable failure code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            SandboxErrorKind::Root => "sandbox_root",
            SandboxErrorKind::Path => "sandbox_path",
            SandboxErrorKind::Io => "sandbox_io",
            SandboxErrorKind::TooLarge => "sandbox_too_large",
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            SandboxErrorKind::Root => f.write_str("the sandbox root must be an existing directory"),
            SandboxErrorKind::Path => {
                f.write_str("sandbox path is not a relative path inside the sandbox root")
            }
            SandboxErrorKind::Io => f.write_str("sandbox file operation failed"),
            SandboxErrorKind::TooLarge => {
                f.write_str("sandbox file exceeds the permitted read bound")
            }
        }
    }
}

impl Error for SandboxError {}

/// A canonicalized sandbox root for relative script file paths.
pub(crate) struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// Canonicalizes `root`, which must already be an existing directory.
    pub(crate) fn new(root: PathBuf) -> Result<Self, SandboxError> {
        let canonical = fs::canonicalize(&root).map_err(|_| SandboxError::root())?;
        let metadata = fs::metadata(&canonical).map_err(|_| SandboxError::root())?;
        if !metadata.is_dir() {
            return Err(SandboxError::root());
        }
        Ok(Self { root: canonical })
    }

    /// Reads one regular file inside the root, bounded by `limit` bytes.
    pub(crate) fn read(&self, relative: &str, limit: usize) -> Result<Vec<u8>, SandboxError> {
        let path = self.resolve_existing(relative)?;
        let metadata = fs::metadata(&path).map_err(|_| SandboxError::io())?;
        if !metadata.is_file() {
            return Err(SandboxError::path());
        }
        let length = usize::try_from(metadata.len()).map_err(|_| SandboxError::too_large())?;
        if length > limit {
            return Err(SandboxError::too_large());
        }
        fs::read(&path).map_err(|_| SandboxError::io())
    }

    /// Writes one regular file inside the root, without following an escaping
    /// symlink or replacing a directory.
    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), SandboxError> {
        let relative = validate_relative(relative)?;
        let target = self.root.join(relative);
        match fs::symlink_metadata(&target) {
            Ok(_) => {
                // The path exists (it may be a symlink, file, directory or
                // special file). Resolve it and require a regular file inside
                // the root; a dangling symlink fails canonicalization.
                let existing = fs::canonicalize(&target).map_err(|_| SandboxError::path())?;
                if !existing.starts_with(&self.root) || existing == self.root {
                    return Err(SandboxError::path());
                }
                let metadata = fs::metadata(&existing).map_err(|_| SandboxError::io())?;
                if !metadata.is_file() {
                    return Err(SandboxError::path());
                }
                fs::write(&existing, bytes).map_err(|_| SandboxError::io())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = target.parent().ok_or_else(SandboxError::path)?;
                let canonical_parent = fs::canonicalize(parent).map_err(|_| SandboxError::io())?;
                if !canonical_parent.starts_with(&self.root) {
                    return Err(SandboxError::path());
                }
                let file_name = target.file_name().ok_or_else(SandboxError::path)?;
                let resolved = canonical_parent.join(file_name);
                if fs::symlink_metadata(&resolved).is_ok() {
                    return Err(SandboxError::path());
                }
                fs::write(&resolved, bytes).map_err(|_| SandboxError::io())
            }
            Err(_) => Err(SandboxError::io()),
        }
    }

    fn resolve_existing(&self, relative: &str) -> Result<PathBuf, SandboxError> {
        let relative = validate_relative(relative)?;
        let joined = self.root.join(relative);
        let canonical = match fs::canonicalize(&joined) {
            Ok(canonical) => canonical,
            Err(_) => {
                return Err(if fs::symlink_metadata(&joined).is_ok() {
                    SandboxError::path()
                } else {
                    SandboxError::io()
                });
            }
        };
        if !canonical.starts_with(&self.root) || canonical == self.root {
            return Err(SandboxError::path());
        }
        Ok(canonical)
    }
}

fn validate_relative(relative: &str) -> Result<PathBuf, SandboxError> {
    if relative.is_empty() || relative.contains('\0') {
        return Err(SandboxError::path());
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(SandboxError::path());
    }
    let mut components = path.components();
    let mut saw_normal = false;
    for component in &mut components {
        match component {
            Component::Normal(_) => saw_normal = true,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(SandboxError::path()),
        }
    }
    if !saw_normal {
        return Err(SandboxError::path());
    }
    Ok(path.to_path_buf())
}
