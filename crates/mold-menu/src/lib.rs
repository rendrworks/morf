//! General hierarchical menu models.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 32;
const MAX_TEXT: usize = 4096;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ButtonType {
    #[default]
    None,
    CheckBox,
    RadioButton,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CheckState {
    #[default]
    Unchecked,
    PartiallyChecked,
    Checked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuEntry {
    pub id: String,
    pub separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub text: String,
    pub icon: Option<String>,
    pub button_type: ButtonType,
    pub check_state: CheckState,
    pub radio_group: Option<String>,
    pub children: Vec<MenuEntry>,
}

impl MenuEntry {
    pub fn item(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            separator: false,
            enabled: true,
            visible: true,
            text: text.into(),
            icon: None,
            button_type: ButtonType::None,
            check_state: CheckState::Unchecked,
            radio_group: None,
            children: Vec::new(),
        }
    }

    pub fn separator(id: impl Into<String>) -> Self {
        let mut entry = Self::item(id, "");
        entry.separator = true;
        entry.enabled = false;
        entry
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Activation {
    pub id: String,
    pub check_state: CheckState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Menu {
    entries: Vec<MenuEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuError {
    EmptyId,
    DuplicateId(String),
    TooManyEntries,
    TooDeep,
    TextTooLong,
    MissingEntry(String),
    NotActivatable(String),
    InvalidButtonState(String),
}

impl fmt::Display for MenuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => formatter.write_str("menu entry IDs cannot be empty"),
            Self::DuplicateId(id) => write!(formatter, "duplicate menu entry ID `{id}`"),
            Self::TooManyEntries => formatter.write_str("menu exceeds 4096 entries"),
            Self::TooDeep => formatter.write_str("menu exceeds 32 levels"),
            Self::TextTooLong => {
                formatter.write_str("menu text, icon, or group exceeds 4096 bytes")
            }
            Self::MissingEntry(id) => write!(formatter, "menu entry `{id}` was not found"),
            Self::NotActivatable(id) => write!(formatter, "menu entry `{id}` is not activatable"),
            Self::InvalidButtonState(id) => {
                write!(
                    formatter,
                    "menu entry `{id}` has a check state without a button"
                )
            }
        }
    }
}

impl Error for MenuError {}

impl Menu {
    pub fn new(entries: Vec<MenuEntry>) -> Result<Self, MenuError> {
        let mut ids = HashSet::new();
        let mut count = 0;
        validate(&entries, 0, &mut count, &mut ids)?;
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    pub fn children(&self, parent: Option<&str>) -> Result<&[MenuEntry], MenuError> {
        match parent {
            None => Ok(&self.entries),
            Some(id) => self
                .entry(id)
                .map(|entry| entry.children.as_slice())
                .ok_or_else(|| MenuError::MissingEntry(id.to_owned())),
        }
    }

    pub fn entry(&self, id: &str) -> Option<&MenuEntry> {
        find(&self.entries, id)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), MenuError> {
        find_mut(&mut self.entries, id)
            .ok_or_else(|| MenuError::MissingEntry(id.to_owned()))?
            .enabled = enabled;
        Ok(())
    }

    pub fn set_visible(&mut self, id: &str, visible: bool) -> Result<(), MenuError> {
        find_mut(&mut self.entries, id)
            .ok_or_else(|| MenuError::MissingEntry(id.to_owned()))?
            .visible = visible;
        Ok(())
    }

    pub fn set_check_state(&mut self, id: &str, state: CheckState) -> Result<(), MenuError> {
        let entry = find_mut(&mut self.entries, id)
            .ok_or_else(|| MenuError::MissingEntry(id.to_owned()))?;
        if entry.button_type == ButtonType::None && state != CheckState::Unchecked {
            return Err(MenuError::InvalidButtonState(id.to_owned()));
        }
        entry.check_state = state;
        Ok(())
    }

    pub fn activate(&mut self, id: &str) -> Result<Activation, MenuError> {
        let (enabled, visible, separator, button_type, group) = self
            .entry(id)
            .map(|entry| {
                (
                    entry.enabled,
                    entry.visible,
                    entry.separator,
                    entry.button_type,
                    entry.radio_group.clone(),
                )
            })
            .ok_or_else(|| MenuError::MissingEntry(id.to_owned()))?;
        if !enabled || !visible || separator {
            return Err(MenuError::NotActivatable(id.to_owned()));
        }
        if button_type == ButtonType::RadioButton && group.is_some() {
            uncheck_radio_group(&mut self.entries, group.as_deref(), id);
        }
        let entry = find_mut(&mut self.entries, id).expect("entry was found above");
        entry.check_state = match button_type {
            ButtonType::None => CheckState::Unchecked,
            ButtonType::CheckBox => match entry.check_state {
                CheckState::Checked => CheckState::Unchecked,
                CheckState::Unchecked | CheckState::PartiallyChecked => CheckState::Checked,
            },
            ButtonType::RadioButton => CheckState::Checked,
        };
        Ok(Activation {
            id: id.to_owned(),
            check_state: entry.check_state,
        })
    }
}

fn validate(
    entries: &[MenuEntry],
    depth: usize,
    count: &mut usize,
    ids: &mut HashSet<String>,
) -> Result<(), MenuError> {
    if depth >= MAX_DEPTH && !entries.is_empty() {
        return Err(MenuError::TooDeep);
    }
    for entry in entries {
        *count += 1;
        if *count > MAX_ENTRIES {
            return Err(MenuError::TooManyEntries);
        }
        if entry.id.is_empty() {
            return Err(MenuError::EmptyId);
        }
        if !ids.insert(entry.id.clone()) {
            return Err(MenuError::DuplicateId(entry.id.clone()));
        }
        if entry.text.len() > MAX_TEXT
            || entry
                .icon
                .as_ref()
                .is_some_and(|icon| icon.len() > MAX_TEXT)
            || entry
                .radio_group
                .as_ref()
                .is_some_and(|group| group.len() > MAX_TEXT)
        {
            return Err(MenuError::TextTooLong);
        }
        if entry.button_type == ButtonType::None && entry.check_state != CheckState::Unchecked {
            return Err(MenuError::InvalidButtonState(entry.id.clone()));
        }
        validate(&entry.children, depth + 1, count, ids)?;
    }
    Ok(())
}

fn find<'a>(entries: &'a [MenuEntry], id: &str) -> Option<&'a MenuEntry> {
    entries.iter().find_map(|entry| {
        if entry.id == id {
            Some(entry)
        } else {
            find(&entry.children, id)
        }
    })
}

fn find_mut<'a>(entries: &'a mut [MenuEntry], id: &str) -> Option<&'a mut MenuEntry> {
    for entry in entries {
        if entry.id == id {
            return Some(entry);
        }
        if let Some(found) = find_mut(&mut entry.children, id) {
            return Some(found);
        }
    }
    None
}

fn uncheck_radio_group(entries: &mut [MenuEntry], group: Option<&str>, active: &str) {
    for entry in entries {
        if entry.id != active
            && entry.button_type == ButtonType::RadioButton
            && entry.radio_group.as_deref() == group
        {
            entry.check_state = CheckState::Unchecked;
        }
        uncheck_radio_group(&mut entry.children, group, active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activates_check_and_radio_entries() {
        let mut check = MenuEntry::item("check", "Check");
        check.button_type = ButtonType::CheckBox;
        let mut one = MenuEntry::item("one", "One");
        one.button_type = ButtonType::RadioButton;
        one.radio_group = Some("choice".into());
        one.check_state = CheckState::Checked;
        let mut two = MenuEntry::item("two", "Two");
        two.button_type = ButtonType::RadioButton;
        two.radio_group = Some("choice".into());
        let mut menu = Menu::new(vec![check, one, two]).unwrap();

        assert_eq!(
            menu.activate("check").unwrap().check_state,
            CheckState::Checked
        );
        assert_eq!(
            menu.activate("two").unwrap().check_state,
            CheckState::Checked
        );
        assert_eq!(
            menu.entry("one").unwrap().check_state,
            CheckState::Unchecked
        );
    }

    #[test]
    fn exposes_nested_children_and_rejects_duplicates() {
        let mut root = MenuEntry::item("root", "Root");
        root.children.push(MenuEntry::item("child", "Child"));
        let menu = Menu::new(vec![root]).unwrap();
        assert_eq!(menu.children(Some("root")).unwrap()[0].id, "child");
        assert!(matches!(
            Menu::new(vec![MenuEntry::item("x", "A"), MenuEntry::item("x", "B")]),
            Err(MenuError::DuplicateId(id)) if id == "x"
        ));
    }
}
