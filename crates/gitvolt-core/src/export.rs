//! Settings export: a file the user asked for, written into a folder they picked.

use crate::error::{GbError, GbErrorKind};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 1024 * 1024;

/// Refuses a name that could leave the folder, and contents over 1 MiB (the UI shows it as a toast).
pub(crate) fn check(name: &str, contents: &str) -> Result<(), GbError> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(GbError::new(GbErrorKind::Other, format!("\"{name}\" isn't a file name")));
    }
    if contents.len() > MAX_BYTES {
        return Err(GbError::new(GbErrorKind::Other, "That export is over 1 MiB"));
    }
    Ok(())
}

/// Writes `<dir>/<name>` (0644), or `<stem> (2).<ext>`, `(3)`… when taken: never overwrites.
pub(crate) fn write(dir: &Path, name: &str, contents: &str) -> Result<PathBuf, GbError> {
    check(name, contents)?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    for n in 1u32.. {
        let file = if n == 1 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(file);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o644);
        match opts.open(&path) {
            Ok(mut f) => {
                f.write_all(contents.as_bytes()).map_err(|e| GbError::new(GbErrorKind::Other, format!("couldn't write {}: {e}", path.display())))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(GbError::new(GbErrorKind::Other, format!("couldn't write {}: {e}", path.display()))),
        }
    }
    unreachable!()
}
