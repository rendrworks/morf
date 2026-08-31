use std::error::Error as StdError;
use std::fmt;

use xkbcommon::xkb;

/// One symbol produced by an XKB key level.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XkbSymbol {
    /// Numeric XKB keysym.
    pub keysym: u32,
    /// Canonical keysym name.
    pub name: String,
    /// UTF-8 text produced by the symbol.
    pub text: String,
}

/// One physical key and its configured layout levels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XkbKey {
    /// XKB keycode including the evdev offset.
    pub keycode: u32,
    /// Linux evdev keycode accepted by virtual-keyboard-v1.
    pub evdev_code: u32,
    /// Physical XKB key name.
    pub name: String,
    /// Whether holding the key should repeat.
    pub repeats: bool,
    /// Layouts containing levels containing produced symbols.
    pub layouts: Vec<Vec<Vec<XkbSymbol>>>,
}

/// Compiled XKB source and OSK-facing key table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XkbKeymap {
    /// Serialized XKB keymap text.
    pub source: String,
    /// Keys sorted by XKB keycode.
    pub keys: Vec<XkbKey>,
}

/// XKB keymap compilation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XkbError(String);

impl fmt::Display for XkbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl StdError for XkbError {}

impl XkbKeymap {
    /// Compiles XKB names into serialized keymap and per-level labels.
    pub fn compile(
        rules: &str,
        model: &str,
        layout: &str,
        variant: &str,
        options: Option<&str>,
    ) -> Result<Self, XkbError> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            rules,
            model,
            layout,
            variant,
            options.map(str::to_owned),
            xkb::COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| XkbError("could not compile XKB keymap".to_owned()))?;
        let mut keys = Vec::new();
        keymap.key_for_each(|keymap, keycode| {
            let Some(name) = keymap.key_get_name(keycode) else {
                return;
            };
            let mut layouts = Vec::new();
            for layout_index in 0..keymap.num_layouts_for_key(keycode) {
                let mut levels = Vec::new();
                for level in 0..keymap.num_levels_for_key(keycode, layout_index) {
                    levels.push(
                        keymap
                            .key_get_syms_by_level(keycode, layout_index, level)
                            .iter()
                            .map(|symbol| XkbSymbol {
                                keysym: symbol.raw(),
                                name: xkb::keysym_get_name(*symbol),
                                text: xkb::keysym_to_utf8(*symbol),
                            })
                            .collect(),
                    );
                }
                layouts.push(levels);
            }
            let repeats = keymap.key_repeats(keycode);
            let keycode = keycode.raw();
            keys.push(XkbKey {
                keycode,
                evdev_code: keycode.saturating_sub(8),
                name: name.to_owned(),
                repeats,
                layouts,
            });
        });
        keys.sort_by_key(|key| key.keycode);
        Ok(Self {
            source: keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1),
            keys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_osk_labels_and_evdev_codes() {
        let keymap = XkbKeymap::compile("", "pc105", "us", "", None).unwrap();
        let key = keymap.keys.iter().find(|key| key.name == "AC01").unwrap();

        assert_eq!(key.evdev_code + 8, key.keycode);
        assert_eq!(key.layouts[0][0][0].text, "a");
        assert_eq!(key.layouts[0][1][0].text, "A");
        assert!(key.repeats);
        assert!(keymap.source.contains("xkb_keymap"));
    }
}
