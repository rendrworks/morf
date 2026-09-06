// What faces this machine has.
//
// A configuration that lets someone choose a font — a settings panel, a picker,
// a demonstration that wants to show every face in turn — cannot be written
// against a list of names guessed in advance, because the answer is different on
// every machine. It has to ask.

use std::path::PathBuf;
use std::sync::OnceLock;

use cosmic_text::fontdb;

/// The files an installed family is drawn from: for each weight, the face
/// the renderer itself would pick for that family, upright and at normal
/// width.
///
/// For a bundle: a configuration that names a face needs that face on the
/// machine it lands on, and the only way to be sure is to carry it. Asked of
/// the database as a query, weight by weight, rather than as a name match,
/// because one family name is shared by every cut a foundry ships under it —
/// dozens of files for a large monospace — and what a `font_weight` picks
/// among is what should travel.
pub fn family_files(family: &str) -> Vec<PathBuf> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    if let Some(paths) = std::env::var_os("MORF_FONT_PATH") {
        for path in std::env::split_paths(&paths) {
            if path.is_dir() {
                database.load_fonts_dir(&path);
            } else {
                let _ = database.load_font_file(&path);
            }
        }
    }
    let family = family.trim();
    let mut files = Vec::new();
    for weight in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let Some(face) = database.face(id) else {
            continue;
        };
        // The query falls back to any family when the named one is absent;
        // a face that is not the family asked for is not carried.
        if !face
            .families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            continue;
        }
        let path = match &face.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => path.clone(),
            fontdb::Source::Binary(_) => continue,
        };
        if !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// Every font family installed, sorted, without duplicates.
///
/// The same set the renderer draws from: the system fonts, plus anything on
/// `MORF_FONT_PATH`. A face loaded later by a node's own `font_source` is not
/// here, but the configuration that named it already knows about it.
///
/// This builds a font database of its own and drops it again, keeping only the
/// names — a scan of the font directories, so the answer is worked out once on
/// first ask and handed back from then on. Nothing pays for it unless something
/// asks.
pub fn installed_families() -> &'static [String] {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        if let Some(paths) = std::env::var_os("MORF_FONT_PATH") {
            for path in std::env::split_paths(&paths) {
                if path.is_dir() {
                    database.load_fonts_dir(&path);
                } else {
                    let _ = database.load_font_file(&path);
                }
            }
        }
        let mut names: Vec<String> = database
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    })
}
