// What faces this machine has.
//
// A configuration that lets someone choose a font — a settings panel, a picker,
// a demonstration that wants to show every face in turn — cannot be written
// against a list of names guessed in advance, because the answer is different on
// every machine. It has to ask.

use std::sync::OnceLock;

use cosmic_text::fontdb;

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
