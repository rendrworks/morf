//! A bundle: morf and a configuration in one file.
//!
//! `morf bundle greeter.lua -o logre` writes `logre`, a copy of the running
//! `morf` with the configuration appended to it — the file itself, the
//! `lib`, `assets`, `plugin` and `fonts` directories beside it, and every
//! font family the configuration names, found by running it once and reading
//! what its nodes ask for — compressed, with a trailer at the end saying how
//! much was appended. `logre` then needs neither `morf` on the path nor the
//! configuration nor its fonts on disk: it looks at its own tail on start,
//! unpacks what it finds into the runtime directory, and runs that.
//!
//! The format is deliberately plain: an executable with bytes after it still
//! starts, because a loader reads the ELF headers and never the end, and a
//! trailer at the very end is found with one seek. There is no index; the
//! payload is small and read whole.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use morf_lua::Runtime;
use morf_scene::Scene;

use crate::config::LoadPolicy;
use crate::supervisor::execute_config;

/// The last bytes of a bundle: the payload's length, then this.
const MAGIC: &[u8; 8] = b"MORFBNDL";
/// A payload longer than this is not one of ours, whatever the trailer says.
const MAX_PAYLOAD: u64 = 1024 * 1024 * 1024;
/// The directories beside a configuration that are carried with it. `fonts`
/// is also where `--font` puts what it finds, and what morf adds to its font
/// path when it runs a configuration that has one.
const CARRIED: [&str; 4] = ["lib", "assets", "plugin", "fonts"];

/// What went into a bundle, for the report.
pub(crate) struct Bundled {
    pub(crate) output: PathBuf,
    pub(crate) files: usize,
    pub(crate) payload: usize,
}

/// One file in the payload: where it goes under the unpack directory, and
/// its bytes.
struct Entry {
    path: String,
    data: Vec<u8>,
}

/// Writes `output`: this executable, then the configuration and what it
/// carries, then the trailer. The first entry is always the configuration
/// itself, which is how the unpacker knows what to run.
pub(crate) fn write(config: &Path, output: &Path, extras: &[PathBuf]) -> Result<Bundled, String> {
    let config = config
        .canonicalize()
        .map_err(|error| format!("could not read {}: {error}", config.display()))?;
    let root = config
        .parent()
        .ok_or_else(|| "the configuration has no directory".to_owned())?
        .to_path_buf();
    let name = config
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "the configuration's name is not text".to_owned())?
        .to_owned();
    let mut entries = vec![Entry {
        path: name,
        data: fs::read(&config)
            .map_err(|error| format!("could not read {}: {error}", config.display()))?,
    }];
    for carried in CARRIED {
        collect(&root, &root.join(carried), &mut entries)?;
    }
    for extra in extras {
        let extra = extra
            .canonicalize()
            .map_err(|error| format!("could not read {}: {error}", extra.display()))?;
        // Inside the configuration's directory a path keeps its place; from
        // anywhere else it goes in by its own name, which is where a
        // `require` or a `shell_path` of that name will look.
        let base = if extra.starts_with(&root) {
            root.clone()
        } else {
            extra.parent().unwrap_or(&root).to_path_buf()
        };
        collect(&base, &extra, &mut entries)?;
    }
    // The faces the configuration asks for, found by asking it: run once,
    // and every family a node names is one to carry, every weight of it,
    // under `fonts/` by its own name. A family the machine has not got is an
    // error here rather than a fallback on some other machine.
    for family in families_named_by(&config)? {
        let files = morf_text::family_files(&family);
        if files.is_empty() {
            return Err(format!(
                "{} names the font family `{family}`, which is not installed here",
                config.display()
            ));
        }
        for file in files {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("{} has no usable name", file.display()))?;
            let path = format!("fonts/{name}");
            if entries.iter().all(|entry| entry.path != path) {
                entries.push(Entry {
                    path,
                    data: fs::read(&file)
                        .map_err(|error| format!("could not read {}: {error}", file.display()))?,
                });
            }
        }
    }
    let payload = pack(&entries)?;
    let own =
        env::current_exe().map_err(|error| format!("could not find this executable: {error}"))?;
    let mut image =
        fs::read(&own).map_err(|error| format!("could not read {}: {error}", own.display()))?;
    // A bundle made from a bundle would carry two payloads and unpack the
    // older; strip the one that is there.
    if let Some(start) = payload_start(&image) {
        image.truncate(start);
    }
    image.extend_from_slice(&payload);
    image.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    image.extend_from_slice(MAGIC);
    fs::write(output, &image)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    fs::set_permissions(output, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("could not make {} executable: {error}", output.display()))?;
    Ok(Bundled {
        output: output.to_path_buf(),
        files: entries.len(),
        payload: payload.len(),
    })
}

/// The font families a configuration names, found by running it and reading
/// every node's `font_family`. A stack — `"Iosevka, monospace"` — is taken
/// one name at a time, and the generic names are left out: those are the
/// machine's to answer, whichever machine it is.
fn families_named_by(config: &Path) -> Result<BTreeSet<String>, String> {
    let source = fs::read(config)
        .map_err(|error| format!("could not read {}: {error}", config.display()))?;
    let mut runtime = Runtime::default();
    execute_config(&mut runtime, config, &source, LoadPolicy::default())?;
    Ok(families_in(&runtime.scene()))
}

/// Every named family in a scene, generics aside.
fn families_in(scene: &Scene) -> BTreeSet<String> {
    const GENERIC: [&str; 8] = [
        "sans-serif",
        "serif",
        "monospace",
        "cursive",
        "fantasy",
        "system-ui",
        "ui-monospace",
        "ui-sans-serif",
    ];
    let mut families = BTreeSet::new();
    let mut stack = scene.roots();
    while let Some(node) = stack.pop() {
        if let Ok(children) = scene.children(node) {
            stack.extend(children.iter().copied());
        }
        let Ok(stacked) = scene.string_value(node, "font_family") else {
            continue;
        };
        for name in stacked.split(',') {
            let name = name.trim().trim_matches('"').trim_matches('\'');
            if name.is_empty() || GENERIC.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            families.insert(name.to_owned());
        }
    }
    families
}

/// Every file under `path`, as entries named relative to `base`. A path that
/// is a file is one entry; one that is missing is nothing, because `lib` or
/// `assets` beside a configuration is common but not required.
fn collect(base: &Path, path: &Path, entries: &mut Vec<Entry>) -> Result<(), String> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.is_file() {
        let relative = path
            .strip_prefix(base)
            .map_err(|_| format!("{} is not under {}", path.display(), base.display()))?;
        let name = relative
            .to_str()
            .ok_or_else(|| format!("{} is not a text path", relative.display()))?
            .to_owned();
        if entries.iter().all(|entry| entry.path != name) {
            entries.push(Entry {
                path: name,
                data: fs::read(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()))?,
            });
        }
        return Ok(());
    }
    let mut children = fs::read_dir(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        collect(base, &child, entries)?;
    }
    Ok(())
}

/// The entries, serialised and compressed.
fn pack(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut plain = Vec::new();
    for entry in entries {
        plain.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        plain.extend_from_slice(entry.path.as_bytes());
        plain.extend_from_slice(&(entry.data.len() as u64).to_le_bytes());
        plain.extend_from_slice(&entry.data);
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&plain)
        .and_then(|()| encoder.finish())
        .map_err(|error| format!("could not compress the bundle: {error}"))
}

/// The entries back out of a payload.
fn unpack_entries(payload: &[u8]) -> Result<Vec<Entry>, String> {
    let mut plain = Vec::new();
    GzDecoder::new(payload)
        .read_to_end(&mut plain)
        .map_err(|error| format!("the bundle's payload is damaged: {error}"))?;
    let mut entries = Vec::new();
    let mut at = 0;
    let take = |at: &mut usize, count: usize| -> Result<&[u8], String> {
        let end = at
            .checked_add(count)
            .filter(|end| *end <= plain.len())
            .ok_or_else(|| "the bundle's payload is truncated".to_owned())?;
        let slice = &plain[*at..end];
        *at = end;
        Ok(slice)
    };
    while at < plain.len() {
        let name_len =
            u32::from_le_bytes(take(&mut at, 4)?.try_into().expect("four bytes")) as usize;
        let name = String::from_utf8(take(&mut at, name_len)?.to_vec())
            .map_err(|_| "a bundled path is not text".to_owned())?;
        let data_len =
            u64::from_le_bytes(take(&mut at, 8)?.try_into().expect("eight bytes")) as usize;
        let data = take(&mut at, data_len)?.to_vec();
        entries.push(Entry { path: name, data });
    }
    Ok(entries)
}

/// Where a payload begins in an image that ends with our trailer, if it does.
fn payload_start(image: &[u8]) -> Option<usize> {
    let trailer = 8 + MAGIC.len();
    if image.len() < trailer || &image[image.len() - MAGIC.len()..] != MAGIC {
        return None;
    }
    let length_at = image.len() - trailer;
    let length = u64::from_le_bytes(
        image[length_at..length_at + 8]
            .try_into()
            .expect("eight bytes"),
    );
    if length > MAX_PAYLOAD || length as usize > length_at {
        return None;
    }
    Some(length_at - length as usize)
}

/// The payload appended to this executable, if there is one. Reads the tail
/// only: a bundle is a large file and the loader has already paid for the
/// front of it.
pub(crate) fn embedded() -> Result<Option<Vec<u8>>, String> {
    let own =
        env::current_exe().map_err(|error| format!("could not find this executable: {error}"))?;
    let mut file = fs::File::open(&own)
        .map_err(|error| format!("could not open {}: {error}", own.display()))?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let trailer = (8 + MAGIC.len()) as u64;
    if size < trailer {
        return Ok(None);
    }
    let mut tail = [0_u8; 16];
    file.seek(SeekFrom::Start(size - trailer))
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut tail)
        .map_err(|error| error.to_string())?;
    if &tail[8..] != MAGIC {
        return Ok(None);
    }
    let length = u64::from_le_bytes(tail[..8].try_into().expect("eight bytes"));
    if length > MAX_PAYLOAD || length + trailer > size {
        return Err("this bundle's trailer does not match its size".to_owned());
    }
    let mut payload = vec![0_u8; length as usize];
    file.seek(SeekFrom::Start(size - trailer - length))
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut payload)
        .map_err(|error| error.to_string())?;
    Ok(Some(payload))
}

/// Unpacks a payload under the runtime directory and returns the path of
/// the configuration in it. Keyed on the payload's hash, so the same bundle
/// started twice unpacks once, and a changed one never runs stale files.
pub(crate) fn unpack(payload: &[u8]) -> Result<PathBuf, String> {
    unpack_under(payload, &runtime_base())
}

/// The same, under a directory of the caller's choosing.
fn unpack_under(payload: &[u8], base: &Path) -> Result<PathBuf, String> {
    let entries = unpack_entries(payload)?;
    let first = entries
        .first()
        .ok_or_else(|| "the bundle is empty".to_owned())?
        .path
        .clone();
    let dir = base
        .join("morf")
        .join(format!("bundle-{:016x}", fnv1a(payload)));
    let ready = dir.join(".ready");
    if !ready.exists() {
        for entry in &entries {
            let path = dir.join(&entry.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            fs::write(&path, &entry.data)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
        fs::write(&ready, b"")
            .map_err(|error| format!("could not write {}: {error}", ready.display()))?;
    }
    Ok(dir.join(first))
}

/// `$XDG_RUNTIME_DIR`, or a directory of our own under `/tmp` for a process
/// that has no runtime directory, which a greeter run by greetd may not.
fn runtime_base() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp").join(format!("morf-{}", uid())))
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail and touches no memory.
    unsafe { libc::getuid() }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_survive_packing() {
        let entries = vec![
            Entry {
                path: "shell.lua".into(),
                data: b"return 1".to_vec(),
            },
            Entry {
                path: "lib/board.lua".into(),
                data: vec![0, 1, 2, 255],
            },
            Entry {
                path: "assets/empty".into(),
                data: Vec::new(),
            },
        ];
        let payload = pack(&entries).unwrap();
        let back = unpack_entries(&payload).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].path, "shell.lua");
        assert_eq!(back[1].data, vec![0, 1, 2, 255]);
        assert_eq!(back[2].data, Vec::<u8>::new());
    }

    #[test]
    fn the_families_a_configuration_names_are_read_off_its_scene() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "faces.lua",
                br#"
                    local ui = require("morf.ui")
                    ui.Item {
                        ui.Text { text = "a", font_family = "Iosevka Nerd Font Mono" },
                        ui.Text { text = "b", font_family = "Liberation Serif, serif" },
                        ui.Text { text = "c" },
                        ui.Item { ui.Text { text = "d", font_family = "Iosevka Nerd Font Mono" } },
                    }
                "#,
            )
            .unwrap();
        let families = families_in(&runtime.scene());
        assert_eq!(
            families.into_iter().collect::<Vec<_>>(),
            ["Iosevka Nerd Font Mono", "Liberation Serif"]
        );
    }

    #[test]
    fn a_trailer_names_its_payload_and_nothing_else_does() {
        let payload = pack(&[Entry {
            path: "a.lua".into(),
            data: b"x".to_vec(),
        }])
        .unwrap();
        let mut image = b"ELF...".to_vec();
        let start = image.len();
        image.extend_from_slice(&payload);
        image.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        image.extend_from_slice(MAGIC);
        assert_eq!(payload_start(&image), Some(start));
        assert_eq!(payload_start(b"ELF... no trailer"), None);
        // A trailer that claims more than the file holds is not a bundle.
        let mut lying = b"short".to_vec();
        lying.extend_from_slice(&(1_000_000_u64).to_le_bytes());
        lying.extend_from_slice(MAGIC);
        assert_eq!(payload_start(&lying), None);
    }

    #[test]
    fn a_bundle_carries_the_configuration_and_what_stands_beside_it() {
        let dir = std::env::temp_dir().join(format!("morf-bundle-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::create_dir_all(dir.join("assets/deep")).unwrap();
        fs::write(dir.join("shell.lua"), b"local x = 1").unwrap();
        fs::write(dir.join("lib/thing.lua"), b"return {}").unwrap();
        fs::write(dir.join("assets/deep/icon.svg"), b"<svg/>").unwrap();
        fs::write(dir.join("unrelated.txt"), b"not carried").unwrap();
        let mut entries = vec![Entry {
            path: "shell.lua".into(),
            data: fs::read(dir.join("shell.lua")).unwrap(),
        }];
        for carried in CARRIED {
            collect(&dir, &dir.join(carried), &mut entries).unwrap();
        }
        let names = entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["shell.lua", "lib/thing.lua", "assets/deep/icon.svg"]
        );

        // Unpacked, the configuration is the first entry and the rest keep
        // their places under it.
        let payload = pack(&entries).unwrap();
        let unpacked = unpack_under(&payload, &dir.join("runtime")).unwrap();
        assert!(unpacked.ends_with("shell.lua"));
        let root = unpacked.parent().unwrap();
        assert_eq!(fs::read(root.join("lib/thing.lua")).unwrap(), b"return {}");
        assert_eq!(
            fs::read(root.join("assets/deep/icon.svg")).unwrap(),
            b"<svg/>"
        );
        // And the same payload unpacks to the same place.
        assert_eq!(
            unpack_under(&payload, &dir.join("runtime")).unwrap(),
            unpacked
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
