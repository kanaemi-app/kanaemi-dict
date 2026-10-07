//! What ships: the dictionaries and the ranking model kept in the
//! repository, each checked and gathered with the notice of its sources and
//! the license.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use kanaemi_engine::{RankingModel, TextDictionary};
use sha2::{Digest, Sha256};

use crate::{Base, RejectedLines};

/// A file to ship: its path under the distribution and its bytes.
pub type DistFile = (PathBuf, Vec<u8>);

/// The name of the base dictionary, the one the others go behind and the
/// model pairs with.
const BASE: &str = "base";
/// What the record of a file's sources is named after.
const SOURCES_SUFFIX: &str = ".sources.txt";
const DICTIONARY_EXTENSION: &str = ".tsv";
const MODEL: &str = "base.model";
/// The license of the dictionaries and the model, kept beside them.
const LICENSE: &str = "LICENSE";
/// The catalog of the dictionaries, beside their folders.
const CATALOG: &str = "index.json";
/// The version of the catalog's shape, raised when it changes.
const CATALOG_FORMAT: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum DistError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{name}: no record of its sources ({name}{SOURCES_SUFFIX})")]
    Unrecorded { name: String },
    #[error("{name}: its sources record is malformed")]
    MalformedRecord { name: String },
    #[error("{name}: changed since its sources were recorded")]
    Stale { name: String },
    #[error("{name}: {source}")]
    Rejected { name: String, source: RejectedLines },
    #[error("{name}: has the line {line:?} the base dictionary has too")]
    Shared { name: String, line: String },
    #[error("{MODEL}: not trained for {BASE}{DICTIONARY_EXTENSION}")]
    Unpaired,
    #[error("{MODEL}: Kanaemi does not read it: {0}")]
    Model(kanaemi_engine::ModelError),
    #[error("no notice for the source {0}")]
    NoNotice(String),
    #[error("no {BASE}{DICTIONARY_EXTENSION} to ship the others with")]
    NoBase,
}

/// The record of the sources of `target`: its SHA-256, the SHA-256 of the
/// file it is `paired` with, and every source once, in the order of their
/// UTF-8 bytes.
pub fn sources_file(
    target: &[u8],
    paired: Option<&[u8]>,
    sources: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let sources: BTreeSet<String> = sources.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let mut out = format!("# sha256 {}\n", sha256_hex(target));
    if let Some(paired) = paired {
        out.push_str(&format!("# pairs {}\n", sha256_hex(paired)));
    }
    for source in sources {
        out.push_str(&source);
        out.push('\n');
    }
    out
}

struct Record {
    sha256: String,
    pairs: Option<String>,
    sources: Vec<String>,
}

fn parse_record(name: &str, text: &str) -> Result<Record, DistError> {
    let malformed = || DistError::MalformedRecord {
        name: name.to_owned(),
    };
    let mut lines = text.lines();
    let sha256 = lines
        .next()
        .and_then(|l| l.strip_prefix("# sha256 "))
        .ok_or_else(malformed)?
        .to_owned();
    let mut pairs = None;
    let mut sources = Vec::new();
    for line in lines {
        match line.strip_prefix("# pairs ") {
            Some(hex) if sources.is_empty() && pairs.is_none() => pairs = Some(hex.to_owned()),
            Some(_) => return Err(malformed()),
            None if line.is_empty() || line.starts_with('#') => return Err(malformed()),
            None => sources.push(line.to_owned()),
        }
    }
    Ok(Record {
        sha256,
        pairs,
        sources,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The notice of `sources`: each source's notice, read by `read` under the
/// source ID up to its first `:`, in the order of those names, a text shared
/// by several sources once.
pub fn notice(
    sources: &[String],
    read: impl Fn(&str) -> Option<String>,
) -> Result<String, DistError> {
    let names: BTreeSet<&str> = sources
        .iter()
        .map(|s| s.split(':').next().unwrap_or_default())
        .collect();
    let mut texts: Vec<String> = Vec::new();
    for name in names {
        let text = read(name).ok_or_else(|| DistError::NoNotice(name.to_owned()))?;
        if !texts.contains(&text) {
            texts.push(text);
        }
    }
    let mut out = String::from(
        "この辞書は、次の素材を使って作った。素材の文章は辞書に含まない。一覧から取った項目は、それぞれの一覧の条件に従って辞書に含む。\n",
    );
    for text in texts {
        out.push_str(
            "\n----------------------------------------------------------------------\n\n",
        );
        out.push_str(text.trim_end());
        out.push('\n');
    }
    Ok(out)
}

/// Every file to ship of the dictionaries and the model in `dir`, each
/// dictionary in a folder of its name with its notice and the license, and
/// the model in the base dictionary's, with the catalog of them all. Anything
/// that would not ship is an error, and then nothing is given.
pub fn gather(dir: &Path, notices: &Path) -> Result<Vec<DistFile>, DistError> {
    let read = |name: &str| {
        let path = dir.join(name);
        std::fs::read(&path).map_err(|source| DistError::Io { path, source })
    };
    let license = read(LICENSE)?;
    let read_notice =
        |name: &str| std::fs::read_to_string(notices.join(format!("{name}.txt"))).ok();
    let mut names = dictionary_names(dir)?;
    if !names.iter().any(|n| n == BASE) {
        return Err(DistError::NoBase);
    }
    names.sort_by_key(|n| n != BASE);
    let mut files: Vec<DistFile> = Vec::new();
    let mut catalog: Vec<serde_json::Value> = Vec::new();
    let mut base: Option<(Vec<u8>, Base)> = None;
    for name in names {
        let file = format!("{name}{DICTIONARY_EXTENSION}");
        let bytes = read(&file)?;
        let mut sources = checked(&file, &bytes, &read)?.sources;
        let text = String::from_utf8_lossy(&bytes);
        let (_, invalid) = TextDictionary::parse(&bytes);
        if !invalid.is_empty() {
            return Err(DistError::Rejected {
                name: file,
                source: RejectedLines(invalid),
            });
        }
        let folder = PathBuf::from(&name);
        let shipped = format!("kanaemi-{name}{DICTIONARY_EXTENSION}");
        let mut entry = serde_json::json!({
            "name": name,
            "base": base.is_none(),
            "label": label(&text).unwrap_or(&name),
            "archive": format!("kanaemi-{name}.zip"),
            "dictionary": {
                "file": shipped,
                "size": bytes.len(),
                "sha256": sha256_hex(&bytes),
            },
        });
        match &base {
            None => {
                let parsed = Base::parse(&text).map_err(|source| DistError::Rejected {
                    name: file.clone(),
                    source,
                })?;
                if dir.join(MODEL).exists() {
                    let model = read(MODEL)?;
                    let record = checked(MODEL, &model, &read)?;
                    if record.pairs.as_deref() != Some(sha256_hex(&bytes).as_str()) {
                        return Err(DistError::Unpaired);
                    }
                    RankingModel::open(dir.join(MODEL)).map_err(DistError::Model)?;
                    entry["model"] = serde_json::json!({
                        "format": model_format(&model),
                        "sha256": sha256_hex(&model),
                    });
                    sources.extend(record.sources);
                    files.push((folder.join("ranking.model"), model));
                }
                base = Some((bytes.clone(), parsed));
            }
            Some((_, parsed)) => {
                if let Some(line) = parsed.shared_line(&text) {
                    return Err(DistError::Shared {
                        name: file,
                        line: line.to_owned(),
                    });
                }
            }
        }
        files.push((
            folder.join("NOTICE"),
            notice(&sources, read_notice)?.into_bytes(),
        ));
        files.push((folder.join(LICENSE), license.clone()));
        files.push((folder.join(shipped), bytes));
        catalog.push(entry);
    }
    let catalog = serde_json::json!({
        "format": CATALOG_FORMAT,
        "dictionaries": catalog,
    });
    let mut catalog = serde_json::to_string_pretty(&catalog).expect("JSON values serialize");
    catalog.push('\n');
    files.push((PathBuf::from(CATALOG), catalog.into_bytes()));
    Ok(files)
}

/// The description on a dictionary's first line.
fn label(text: &str) -> Option<&str> {
    let label = text.lines().next()?.strip_prefix('#')?.trim();
    (!label.is_empty()).then_some(label)
}

/// The format version a model file says it is, which `RankingModel::open`
/// has read.
fn model_format(model: &[u8]) -> u32 {
    u32::from_le_bytes(model[8..12].try_into().expect("a model's header is read"))
}

/// The record of `name`, checked against its `bytes`.
fn checked(
    name: &str,
    bytes: &[u8],
    read: &impl Fn(&str) -> Result<Vec<u8>, DistError>,
) -> Result<Record, DistError> {
    let record = match read(&format!("{name}{SOURCES_SUFFIX}")) {
        Ok(record) => record,
        Err(DistError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Err(DistError::Unrecorded {
                name: name.to_owned(),
            });
        }
        Err(e) => return Err(e),
    };
    let record = parse_record(name, &String::from_utf8_lossy(&record))?;
    if record.sha256 != sha256_hex(bytes) {
        return Err(DistError::Stale {
            name: name.to_owned(),
        });
    }
    Ok(record)
}

/// The names of the dictionaries in `dir`, by their file names.
fn dictionary_names(dir: &Path) -> Result<Vec<String>, DistError> {
    let io = |source| DistError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let file = entry
            .map_err(io)?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Some(name) = file.strip_suffix(DICTIONARY_EXTENSION) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Puts the dictionaries and the model of `from` that have a sources record
/// into `to` in place of those there, once they check as they would ship;
/// `to` keeps its license. Returns the files taken. When they do not check,
/// `to` is left as it was.
pub fn take(from: &Path, to: &Path, notices: &Path) -> Result<Vec<String>, DistError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| DistError::Io { path, source }
    };
    let mut taken = Vec::new();
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let file = entry
            .map_err(io(from))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Some(name) = file.strip_suffix(SOURCES_SUFFIX)
            && from.join(name).exists()
        {
            taken.push(name.to_owned());
        }
    }
    taken.sort();
    // Checked as a whole in a folder of their own beside `to`, then moved in.
    let staged = {
        let mut name = to.as_os_str().to_owned();
        name.push(format!(".{}.tmp", std::process::id()));
        PathBuf::from(name)
    };
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).map_err(io(&staged))?;
    let staging = (|| {
        std::fs::copy(to.join(LICENSE), staged.join(LICENSE)).map_err(io(&to.join(LICENSE)))?;
        for name in &taken {
            for file in [name.clone(), format!("{name}{SOURCES_SUFFIX}")] {
                std::fs::copy(from.join(&file), staged.join(&file))
                    .map_err(io(&from.join(&file)))?;
            }
        }
        gather(&staged, notices)
    })();
    if let Err(e) = staging {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    for entry in std::fs::read_dir(to).map_err(io(to))? {
        let path = entry.map_err(io(to))?.path();
        if path.is_file() && path.file_name().is_some_and(|n| n != LICENSE) {
            std::fs::remove_file(&path).map_err(io(&path))?;
        }
    }
    for entry in std::fs::read_dir(&staged).map_err(io(&staged))? {
        let path = entry.map_err(io(&staged))?.path();
        let target = to.join(path.file_name().expect("a file"));
        std::fs::rename(&path, &target).map_err(io(&target))?;
    }
    std::fs::remove_dir(&staged).map_err(io(&staged))?;
    Ok(taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_file;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kanaemi-dict-dist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const BASE: &str = "# base\nきしゃ\t記者\t\t20\n";
    const RAILWAY: &str = "# railway\nえき\t駅\t\t10\n";

    fn notices(dir: &Path) -> PathBuf {
        let notices = dir.join("notices");
        std::fs::create_dir_all(&notices).unwrap();
        for (name, text) in [
            ("aozora-text", "青空文庫\n"),
            ("wikipedia-ja", "ウィキペディア\n"),
            ("fineweb2-jpn", "FineWeb-2\n"),
            ("fineweb2-jpn-train", "FineWeb-2\n"),
            ("hatena-hotentry", "はてな\n"),
        ] {
            std::fs::write(notices.join(format!("{name}.txt")), text).unwrap();
        }
        notices
    }

    /// A kept file: its name, bytes, the bytes it pairs with, and its sources.
    type Kept<'a> = (&'a str, &'a [u8], Option<&'a [u8]>, &'a [&'a str]);

    /// A directory of dictionaries as the repository keeps them.
    fn kept(dir: &Path, files: &[Kept]) -> PathBuf {
        let kept = dir.join("dictionaries");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("LICENSE"), "CC BY 4.0\n").unwrap();
        for (name, bytes, paired, sources) in files {
            std::fs::write(kept.join(name), bytes).unwrap();
            std::fs::write(
                kept.join(format!("{name}.sources.txt")),
                sources_file(bytes, *paired, sources.iter()),
            )
            .unwrap();
        }
        kept
    }

    fn model() -> Vec<u8> {
        model_file(10, &vec![0.5; 1 << 10])
    }

    fn names(files: &[DistFile]) -> Vec<String> {
        let mut names: Vec<String> = files
            .iter()
            .map(|(path, _)| path.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn file<'a>(files: &'a [DistFile], name: &str) -> &'a [u8] {
        &files
            .iter()
            .find(|(path, _)| path == Path::new(name))
            .unwrap()
            .1
    }

    #[test]
    fn the_sources_record_holds_the_hashes_and_each_source_once_in_order() {
        let record = sources_file(
            b"dict",
            Some(b"base"),
            ["wikipedia-ja", "aozora-text", "wikipedia-ja"],
        );

        assert_eq!(
            record,
            format!(
                "# sha256 {}\n# pairs {}\naozora-text\nwikipedia-ja\n",
                sha256_hex(b"dict"),
                sha256_hex(b"base"),
            )
        );
    }

    #[test]
    fn a_notice_shows_each_source_kind_once_and_each_text_once() {
        let notice = notice(
            &[
                "aozora-text".into(),
                "fineweb2-jpn".into(),
                "fineweb2-jpn-train".into(),
                "hatena-hotentry:20260101".into(),
                "hatena-hotentry:20260102".into(),
            ],
            |name| {
                Some(match name {
                    "fineweb2-jpn" | "fineweb2-jpn-train" => "FineWeb-2\n".into(),
                    other => format!("{other}\n"),
                })
            },
        )
        .unwrap();

        assert_eq!(notice.matches("FineWeb-2").count(), 1);
        assert_eq!(notice.matches("hatena-hotentry").count(), 1);
        assert!(notice.find("aozora-text") < notice.find("FineWeb-2"));
        let preface = notice.lines().next().unwrap();
        assert!(preface.contains("素材の文章は辞書に含まない"));
        assert!(preface.contains("一覧から取った項目"));
    }

    #[test]
    fn a_source_without_its_notice_is_an_error_naming_it() {
        let err = notice(&["kokkai".into()], |_| None).unwrap_err();

        assert!(err.to_string().contains("kokkai"), "{err}");
    }

    #[test]
    fn each_dictionary_ships_with_its_notice_and_the_license_and_the_base_with_the_model() {
        let dir = scratch("gather");
        let model = model();
        let kept = kept(
            &dir,
            &[
                ("base.tsv", BASE.as_bytes(), None, &["aozora-text"]),
                (
                    "base.model",
                    &model,
                    Some(BASE.as_bytes()),
                    &["fineweb2-jpn"],
                ),
                ("railway.tsv", RAILWAY.as_bytes(), None, &["wikipedia-ja"]),
            ],
        );

        let files = gather(&kept, &notices(&dir)).unwrap();

        assert_eq!(
            names(&files),
            [
                "base/LICENSE",
                "base/NOTICE",
                "base/kanaemi-base.tsv",
                "base/ranking.model",
                "index.json",
                "railway/LICENSE",
                "railway/NOTICE",
                "railway/kanaemi-railway.tsv",
            ]
        );
        assert_eq!(file(&files, "base/kanaemi-base.tsv"), BASE.as_bytes());
        assert_eq!(file(&files, "base/ranking.model"), &model[..]);
        assert_eq!(file(&files, "railway/LICENSE"), b"CC BY 4.0\n");
        let base_notice = String::from_utf8(file(&files, "base/NOTICE").to_vec()).unwrap();
        assert!(base_notice.contains("青空文庫") && base_notice.contains("FineWeb-2"));
        let railway_notice = String::from_utf8(file(&files, "railway/NOTICE").to_vec()).unwrap();
        assert!(railway_notice.contains("ウィキペディア") && !railway_notice.contains("青空文庫"));
    }

    #[test]
    fn the_catalog_lists_each_dictionary_with_what_its_archive_holds() {
        let dir = scratch("catalog");
        let model = model();
        let kept = kept(
            &dir,
            &[
                ("base.tsv", BASE.as_bytes(), None, &["aozora-text"]),
                (
                    "base.model",
                    &model,
                    Some(BASE.as_bytes()),
                    &["fineweb2-jpn"],
                ),
                ("railway.tsv", RAILWAY.as_bytes(), None, &["wikipedia-ja"]),
            ],
        );

        let files = gather(&kept, &notices(&dir)).unwrap();

        let catalog: serde_json::Value =
            serde_json::from_slice(file(&files, "index.json")).unwrap();
        assert_eq!(
            catalog,
            serde_json::json!({
                "format": 1,
                "dictionaries": [
                    {
                        "name": "base",
                        "base": true,
                        "label": "base",
                        "archive": "kanaemi-base.zip",
                        "dictionary": {
                            "file": "kanaemi-base.tsv",
                            "size": BASE.len(),
                            "sha256": sha256_hex(BASE.as_bytes()),
                        },
                        "model": { "format": 3, "sha256": sha256_hex(&model) },
                    },
                    {
                        "name": "railway",
                        "base": false,
                        "label": "railway",
                        "archive": "kanaemi-railway.zip",
                        "dictionary": {
                            "file": "kanaemi-railway.tsv",
                            "size": RAILWAY.len(),
                            "sha256": sha256_hex(RAILWAY.as_bytes()),
                        },
                    },
                ],
            })
        );
    }

    #[test]
    fn a_dictionary_changed_since_its_sources_were_recorded_does_not_ship() {
        let dir = scratch("stale");
        let kept = kept(
            &dir,
            &[("base.tsv", BASE.as_bytes(), None, &["aozora-text"])],
        );
        std::fs::write(kept.join("base.tsv"), "# base\nきしゃ\t汽車\t\t20\n").unwrap();

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("base.tsv"), "{err}");
    }

    #[test]
    fn a_dictionary_without_its_sources_record_does_not_ship() {
        let dir = scratch("unrecorded");
        let kept = kept(
            &dir,
            &[("base.tsv", BASE.as_bytes(), None, &["aozora-text"])],
        );
        std::fs::write(kept.join("music.tsv"), "# music\n").unwrap();

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("music.tsv"), "{err}");
    }

    #[test]
    fn a_dictionary_with_a_line_kanaemi_rejects_does_not_ship() {
        let dir = scratch("rejected");
        let broken: &[u8] = b"# base\n\tno reading\n";
        let kept = kept(&dir, &[("base.tsv", broken, None, &["aozora-text"])]);

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("base.tsv"), "{err}");
    }

    #[test]
    fn an_additional_dictionary_with_a_line_of_the_base_does_not_ship() {
        let dir = scratch("shared");
        let shared = format!("{RAILWAY}きしゃ\t記者\t\t5\n");
        let kept = kept(
            &dir,
            &[
                ("base.tsv", BASE.as_bytes(), None, &["aozora-text"]),
                ("railway.tsv", shared.as_bytes(), None, &["wikipedia-ja"]),
            ],
        );

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("railway.tsv"), "{err}");
    }

    #[test]
    fn a_model_not_paired_with_the_base_does_not_ship() {
        let dir = scratch("unpaired");
        let model = model();
        let kept = kept(
            &dir,
            &[
                ("base.tsv", BASE.as_bytes(), None, &["aozora-text"]),
                (
                    "base.model",
                    &model,
                    Some(b"another base"),
                    &["fineweb2-jpn"],
                ),
            ],
        );

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("base.model"), "{err}");
    }

    #[test]
    fn a_model_kanaemi_does_not_read_does_not_ship() {
        let dir = scratch("unreadable");
        let kept = kept(
            &dir,
            &[
                ("base.tsv", BASE.as_bytes(), None, &["aozora-text"]),
                (
                    "base.model",
                    b"not a model",
                    Some(BASE.as_bytes()),
                    &["fineweb2-jpn"],
                ),
            ],
        );

        let err = gather(&kept, &notices(&dir)).unwrap_err();

        assert!(err.to_string().contains("base.model"), "{err}");
    }

    #[test]
    fn taking_replaces_the_kept_set_with_the_recorded_builds_alone() {
        let dir = scratch("take");
        let notices = notices(&dir);
        let to = kept(&dir, &[("base.tsv", b"# old\n", None, &["aozora-text"])]);
        std::fs::write(to.join("music.tsv"), "# gone\n").unwrap();
        std::fs::write(to.join("music.tsv.sources.txt"), "# sha256 x\n").unwrap();
        let from = dir.join("build");
        std::fs::create_dir_all(&from).unwrap();
        for (name, bytes, sources) in [
            ("base.tsv", BASE.as_bytes(), "aozora-text"),
            ("railway.tsv", RAILWAY.as_bytes(), "wikipedia-ja"),
        ] {
            std::fs::write(from.join(name), bytes).unwrap();
            std::fs::write(
                from.join(format!("{name}.sources.txt")),
                sources_file(bytes, None, [sources]),
            )
            .unwrap();
        }
        std::fs::write(from.join("base-train.tsv"), "# train\n").unwrap();
        std::fs::write(from.join("base-report.tsv"), "kind\n").unwrap();

        let taken = take(&from, &to, &notices).unwrap();

        assert_eq!(taken, ["base.tsv", "railway.tsv"]);
        let mut left: Vec<String> = std::fs::read_dir(&to)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "LICENSE",
                "base.tsv",
                "base.tsv.sources.txt",
                "railway.tsv",
                "railway.tsv.sources.txt"
            ]
        );
        assert_eq!(std::fs::read(to.join("base.tsv")).unwrap(), BASE.as_bytes());
    }

    #[test]
    fn taking_builds_that_do_not_check_leaves_the_kept_set_as_it_was() {
        let dir = scratch("take-bad");
        let notices = notices(&dir);
        let to = kept(
            &dir,
            &[("base.tsv", BASE.as_bytes(), None, &["aozora-text"])],
        );
        let from = dir.join("build");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("base.tsv"), "# new\n").unwrap();
        std::fs::write(
            from.join("base.tsv.sources.txt"),
            sources_file(b"# new\n", None, ["kokkai"]),
        )
        .unwrap();

        assert!(take(&from, &to, &notices).is_err());

        assert_eq!(std::fs::read(to.join("base.tsv")).unwrap(), BASE.as_bytes());
    }
}
