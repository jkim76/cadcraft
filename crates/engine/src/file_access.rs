//! Folders an automation session may read and write.
//!
//! An agent driving `cadcraft-cli mcp` acts with the user's permissions, and every command that
//! names a file (OPEN, SAVEAS, EXPORT, INSERT, XATTACH, IMAGEATTACH, SCRIPT, APPLOAD, AutoLISP's
//! `open`, a render to a path…) reaches the file system. [`confine`] limits that to the folders the
//! person chose when they started the server: reads are allowed inside a read or a write root,
//! writes only inside a write root. Without a call to [`confine`] nothing is checked (the desktop
//! app and the plain CLI keep working as before).
//!
//! The checks sit in the few places files are opened ([`crate::capped_read::read_capped`],
//! [`crate::atomic_write`] and the commands that read or write directly), so the command line,
//! scripts, AutoLISP and JSON commands are confined alike. A path is resolved before it is
//! compared: relative to the working directory, symlinks followed for the part that exists, and
//! the part that doesn't exist yet may hold no `..`. A dangling symlink is refused. Once set, the
//! roots stay for the life of the process.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The folders a confined session may use, each canonical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileRoots {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl FileRoots {
    /// Roots from the folders given: each must be an existing folder. `read` folders may be read;
    /// `write` folders may be read and written.
    pub fn new(read: &[PathBuf], write: &[PathBuf]) -> Result<Self, String> {
        let canon = |p: &PathBuf| -> Result<PathBuf, String> {
            let c = std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))?;
            if c.is_dir() { Ok(c) } else { Err(format!("{}: not a folder", p.display())) }
        };
        Ok(Self { read: read.iter().map(canon).collect::<Result<_, _>>()?, write: write.iter().map(canon).collect::<Result<_, _>>()? })
    }

    /// `Ok` when `path` is inside a read or a write root.
    pub fn check_read(&self, path: &Path) -> io::Result<()> {
        let p = resolve(path)?;
        if self.read.iter().chain(&self.write).any(|r| p.starts_with(r)) { Ok(()) } else { Err(refused(path, "read", &self.read, &self.write)) }
    }

    /// `Ok` when `path` is inside a write root.
    pub fn check_write(&self, path: &Path) -> io::Result<()> {
        let p = resolve(path)?;
        if self.write.iter().any(|r| p.starts_with(r)) { Ok(()) } else { Err(refused(path, "write", &[], &self.write)) }
    }
}

fn refused(path: &Path, what: &str, read: &[PathBuf], write: &[PathBuf]) -> io::Error {
    let list: Vec<String> = read.iter().chain(write).map(|r| r.display().to_string()).collect();
    let allowed = if list.is_empty() { "none".to_string() } else { list.join(", ") };
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} is outside the folders this session may {what} while file access is confined (allowed: {allowed})", path.display()),
    )
}

/// `path` made absolute with symlinks resolved: the longest existing prefix is canonicalized and
/// the rest, which must be plain names, appended.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let mut existing = abs.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    // Bounded by the number of components.
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut out) => {
                out.extend(rest.iter().rev());
                return Ok(out);
            }
            Err(e) => {
                if std::fs::symlink_metadata(existing).is_ok() {
                    // It exists but can't be resolved: a dangling or looping symlink, or no access.
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: can't be resolved ({e})", existing.display())));
                }
                match (existing.parent(), existing.components().next_back()) {
                    (Some(parent), Some(Component::Normal(name))) => {
                        rest.push(name);
                        existing = parent;
                    }
                    _ => {
                        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: can't be resolved", path.display())));
                    }
                }
            }
        }
    }
}

static ROOTS: OnceLock<FileRoots> = OnceLock::new();

/// Confine every later file access of this process to `roots`. A second call with other roots is
/// an error: confinement is never loosened.
pub fn confine(roots: FileRoots) -> Result<(), String> {
    let set = ROOTS.get_or_init(|| roots.clone());
    if *set == roots { Ok(()) } else { Err("file access is already confined to other folders".into()) }
}

/// Whether [`confine`] was called.
pub fn is_confined() -> bool {
    ROOTS.get().is_some()
}

/// `Ok` unless file access is confined and `path` is outside the readable folders.
pub fn check_read(path: &Path) -> io::Result<()> {
    ROOTS.get().map_or(Ok(()), |r| r.check_read(path))
}

/// `Ok` unless file access is confined and `path` is outside the writable folders.
pub fn check_write(path: &Path) -> io::Result<()> {
    ROOTS.get().map_or(Ok(()), |r| r.check_write(path))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tree() -> (crate::temp_dir::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = crate::temp_dir::TempDir::new("file-access");
        let (read, write, other) = (dir.join("in"), dir.join("out"), dir.join("other"));
        for d in [&read, &write, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(other.join("secret.dxf"), b"x").unwrap();
        (dir, read, write, other)
    }

    #[test]
    fn reads_inside_read_or_write_roots_and_writes_only_inside_write_roots() {
        let (_dir, read, write, other) = tree();
        let roots = FileRoots::new(std::slice::from_ref(&read), std::slice::from_ref(&write)).unwrap();
        assert!(roots.check_read(&read.join("a.dxf")).is_ok());
        assert!(roots.check_read(&write.join("new/b.dxf")).is_ok());
        assert!(roots.check_read(&other.join("secret.dxf")).is_err());
        assert!(roots.check_write(&write.join("new/deeper/c.dxf")).is_ok());
        let e = roots.check_write(&read.join("a.dxf")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert!(e.to_string().contains("while file access is confined"), "{e}");
        assert!(roots.check_write(Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn dot_dot_and_symlinks_cannot_leave_a_root() {
        let (_dir, read, write, other) = tree();
        let roots = FileRoots::new(&[read], std::slice::from_ref(&write)).unwrap();
        // `..` through an existing folder resolves outside; through a missing one it is refused.
        assert!(roots.check_write(&write.join("../other/x.dxf")).is_err());
        assert!(roots.check_write(&write.join("missing/../../other/x.dxf")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&other, write.join("link")).unwrap();
            assert!(roots.check_read(&write.join("link/secret.dxf")).is_err());
            std::os::unix::fs::symlink(other.join("nothing.dxf"), write.join("dangling.dxf")).unwrap();
            assert!(roots.check_write(&write.join("dangling.dxf")).is_err());
        }
    }

    #[test]
    fn roots_must_be_existing_folders() {
        let (_dir, read, _write, other) = tree();
        assert!(FileRoots::new(&[read.join("missing")], &[]).is_err());
        assert!(FileRoots::new(&[other.join("secret.dxf")], &[]).is_err());
        assert!(FileRoots::new(&[], &[]).unwrap().check_read(&read).is_err());
    }
}
