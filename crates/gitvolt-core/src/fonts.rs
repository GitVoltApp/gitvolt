//! The system's font families, for the font pickers (Settings › Appearance › Fonts, Diffs and
//! editors › Editor). `fontdb` reads the font files' own name and `post` tables from the
//! platform's font folders (on Linux, the ones fontconfig's configuration lists): no system
//! library, and nothing but names leaves this module.

use serde::Serialize;
use std::path::Path;
use ts_rs::TS;

/// One family, as CSS names it (the font's typographic family, in English where it has one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontFamily {
    pub name: String,
    /// A face of it says it's fixed-pitch (`post.isFixedPitch`): the editor picker lists these.
    pub monospace: bool,
}

/// `db`'s families, sorted case-insensitively, each once (CSS matches family names without
/// regard to case): monospace when any of its faces is. Hidden ones (macOS's `.SF NS`, named
/// with a leading dot) are left out.
pub fn families(db: &fontdb::Database) -> Vec<FontFamily> {
    let mut all: Vec<FontFamily> = db
        .faces()
        .filter_map(|face| {
            let name = face.families.first()?.0.trim();
            (!name.is_empty() && !name.starts_with('.')).then(|| FontFamily { name: name.to_string(), monospace: face.monospaced })
        })
        .collect();
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
    let mut out: Vec<FontFamily> = Vec::with_capacity(all.len());
    for f in all {
        match out.last_mut() {
            Some(last) if last.name.to_lowercase() == f.name.to_lowercase() => last.monospace |= f.monospace,
            _ => out.push(f),
        }
    }
    out
}

/// The installed families: the system's font folders and the user's. Reads every font file's
/// header (memory-mapped), so it runs on the blocking pool, once per session unless refreshed.
pub fn system_families() -> Vec<FontFamily> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    families(&db)
}

/// The families of the fonts in `dir` and its subfolders (tests, a fixed harness list).
pub fn dir_families(dir: &Path) -> Vec<FontFamily> {
    let mut db = fontdb::Database::new();
    db.load_fonts_dir(dir);
    families(&db)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font file with only what `fontdb` reads: a `name` table (the family, as the typographic
    /// family when `typographic`, else as the legacy one, plus the PostScript name it requires)
    /// and a `post` table whose `isFixedPitch` is `mono`.
    fn font(family: &str, typographic: bool, mono: bool) -> Vec<u8> {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let ps: String = family.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let strings = [(if typographic { 16u16 } else { 1u16 }, utf16(family)), (6, utf16(&ps))];
        let mut name: Vec<u8> = Vec::new();
        name.extend(0u16.to_be_bytes()); // format
        name.extend((strings.len() as u16).to_be_bytes());
        name.extend((6 + 12 * strings.len() as u16).to_be_bytes()); // string storage offset
        let mut storage: Vec<u8> = Vec::new();
        for (id, s) in &strings {
            // Windows, Unicode BMP, English (United States).
            for v in [3u16, 1, 0x0409, *id, s.len() as u16, storage.len() as u16] {
                name.extend(v.to_be_bytes());
            }
            storage.extend(s);
        }
        name.extend(storage);
        let mut post = vec![0u8; 32];
        post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        post[12..16].copy_from_slice(&u32::from(mono).to_be_bytes());
        let tables: [(&[u8; 4], Vec<u8>); 2] = [(b"name", name), (b"post", post)];
        let mut out: Vec<u8> = Vec::new();
        out.extend(0x0001_0000u32.to_be_bytes());
        for v in [tables.len() as u16, 32, 1, 0] {
            out.extend(v.to_be_bytes());
        }
        let mut offset = 12 + 16 * tables.len();
        let mut body: Vec<u8> = Vec::new();
        for (tag, data) in &tables {
            out.extend(*tag);
            out.extend(0u32.to_be_bytes()); // checksum: fontdb doesn't check it
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            let padded = data.len().div_ceil(4) * 4;
            body.extend(data);
            body.resize(body.len() + padded - data.len(), 0);
            offset += padded;
        }
        out.extend(body);
        out
    }

    fn write(dir: &Path, file: &str, family: &str, typographic: bool, mono: bool) {
        std::fs::write(dir.join(file), font(family, typographic, mono)).unwrap();
    }

    #[test]
    fn lists_each_family_once_sorted_with_its_monospace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("user");
        std::fs::create_dir(&sub).unwrap();
        write(dir.path(), "zeta.ttf", "Zeta Sans", true, false);
        write(dir.path(), "orbit-regular.ttf", "Orbit Mono", true, true);
        write(dir.path(), "orbit-bold.otf", "Orbit Mono", true, true);
        // Another spelling of the same family: CSS matches case-insensitively.
        write(&sub, "orbit-italic.ttf", "orbit mono", false, true);
        write(&sub, "alpha.TTF", "alpha Serif", false, false);
        write(dir.path(), "hidden.ttf", ".Hidden UI", true, false);
        std::fs::write(dir.path().join("notes.txt"), "not a font").unwrap();
        std::fs::write(dir.path().join("broken.ttf"), b"nope").unwrap();

        let list = dir_families(dir.path());
        assert_eq!(
            list,
            vec![
                FontFamily { name: "alpha Serif".into(), monospace: false },
                FontFamily { name: "Orbit Mono".into(), monospace: true },
                FontFamily { name: "Zeta Sans".into(), monospace: false },
            ]
        );
    }

    #[test]
    fn a_family_is_monospace_when_any_face_is() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.ttf", "Mixed", true, false);
        write(dir.path(), "b.ttf", "Mixed", true, true);
        assert_eq!(dir_families(dir.path()), vec![FontFamily { name: "Mixed".into(), monospace: true }]);
    }

    #[test]
    fn serialises_camel_case() {
        let json = serde_json::to_value(FontFamily { name: "Orbit Mono".into(), monospace: true }).unwrap();
        assert_eq!(json, serde_json::json!({ "name": "Orbit Mono", "monospace": true }));
    }

    #[test]
    fn an_empty_or_missing_folder_has_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dir_families(dir.path()).is_empty());
        assert!(dir_families(&dir.path().join("missing")).is_empty());
    }
}
