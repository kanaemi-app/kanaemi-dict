//! Writing build outputs so that a failed run never leaves a partial file
//! under the real name.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("{}: {source}", path.display())]
pub struct WriteError {
    pub path: PathBuf,
    pub source: io::Error,
}

/// Writes `path` with `write` into a file beside it, and renames that file
/// into place once `write` succeeds and everything is flushed. On failure the
/// file beside it is removed and `path` is left as it was.
pub fn write_atomically<T, E>(
    path: impl AsRef<Path>,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<WriteError>,
{
    let path = path.as_ref();
    let failed = |path: &Path| {
        let path = path.to_path_buf();
        move |source| WriteError { path, source }
    };
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(failed(dir))?;
    }
    let partial = {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{}.tmp", std::process::id()));
        PathBuf::from(name)
    };
    let mut writer = BufWriter::new(File::create(&partial).map_err(failed(&partial))?);
    let written = write(&mut writer).and_then(|value| {
        writer.flush().map_err(failed(&partial))?;
        Ok(value)
    });
    drop(writer);
    let renamed = written.and_then(|value| {
        std::fs::rename(&partial, path).map_err(failed(path))?;
        Ok(value)
    });
    if renamed.is_err() {
        // Best effort: the error being returned matters more than this one.
        let _ = std::fs::remove_file(&partial);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanaemi-dict-output-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[derive(Debug, thiserror::Error)]
    enum Failure {
        #[error(transparent)]
        Io(#[from] io::Error),
        #[error(transparent)]
        Write(#[from] WriteError),
        #[error("given up")]
        GivenUp,
    }

    #[test]
    fn the_file_and_its_directories_are_written_whole() {
        let dir = scratch("whole");
        let path = dir.join("a/b/out.txt");

        let value = write_atomically(&path, |w| {
            w.write_all(b"hello")?;
            Ok::<_, Failure>(42)
        })
        .unwrap();

        assert_eq!(value, 42);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_and_nothing_beside_it() {
        let dir = scratch("failed");
        let path = dir.join("out.txt");
        write_atomically(&path, |w| Ok::<_, Failure>(w.write_all(b"before")?)).unwrap();

        let result = write_atomically(&path, |w| {
            w.write_all(b"after")?;
            Err::<(), _>(Failure::GivenUp)
        });

        assert!(matches!(result, Err(Failure::GivenUp)), "{result:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
