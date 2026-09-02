//! What a desktop entry says, and what it takes to believe it.

use super::*;

#[test]
fn parses_application_actions_and_exec_fields() {
    let entry = DesktopEntry::parse(
        "browser",
        "[Desktop Entry]\nType=Application\nName=Browser\nGenericName=Web Browser\nExec=browser --new-window %U --title 'A B' %%\nIcon=browser\nCategories=Network;WebBrowser;\nActions=Private;\n\n[Desktop Action Private]\nName=Private Window\nExec=browser --private %U\n",
    )
    .unwrap();
    assert_eq!(
        entry.command,
        ["browser", "--new-window", "--title", "A B", "%"]
    );
    assert_eq!(entry.categories, ["Network", "WebBrowser"]);
    assert_eq!(entry.actions[0].command, ["browser", "--private"]);
}

/// A session whose program is not installed is worse than no entry at all:
/// it is offered, it takes a password, and then the screen goes black with
/// nothing to say why. `TryExec` is where a desktop entry admits this.
#[test]
fn an_entry_naming_a_program_that_is_not_there_is_not_offered() {
    let root = std::env::temp_dir().join(format!("morf-tryexec-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("missing.desktop"),
        "[Desktop Entry]\nType=Application\nName=Missing\nExec=nope\nTryExec=morf-no-such-program\n",
    )
    .unwrap();
    fs::write(
        root.join("present.desktop"),
        "[Desktop Entry]\nType=Application\nName=Present\nExec=sh\nTryExec=sh\nDesktopNames=Present;Thing;\n",
    )
    .unwrap();
    let entries = DesktopEntries::scan_paths([root.clone()]).unwrap();
    let names: Vec<&str> = entries
        .applications()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["Present"]);
    let present = &entries.applications()[0];
    assert_eq!(present.desktop_names, ["Present", "Thing"]);
    // Where it was found, which is what says Wayland or X11.
    assert_eq!(present.source, root.to_string_lossy());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn earlier_paths_mask_later_and_hidden_entries() {
    let root = std::env::temp_dir().join(format!("morf-desktop-{}", std::process::id()));
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(
        first.join("masked.desktop"),
        "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\n",
    )
    .unwrap();
    fs::write(
        second.join("masked.desktop"),
        "[Desktop Entry]\nType=Application\nName=Visible\n",
    )
    .unwrap();
    fs::write(
        second.join("shown.desktop"),
        "[Desktop Entry]\nType=Application\nName=Shown\nStartupWMClass=shown\n",
    )
    .unwrap();
    fs::write(
        second.join("internal.desktop"),
        "[Desktop Entry]\nType=Application\nName=Internal\nNoDisplay=true\n",
    )
    .unwrap();
    let entries = DesktopEntries::scan_paths([first, second]).unwrap();
    assert!(entries.by_id("masked").is_none());
    assert!(entries.by_id("internal").is_none());
    assert_eq!(entries.heuristic_lookup("shown").unwrap().name, "Shown");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_session_entry_parses_into_something_runnable() {
    // What a greeter needs from one of these: a name to show, and a command
    // to hand to `greetd`. The file format is the same as an application's,
    // which is the reason this crate can serve both — but it lives in a
    // directory this crate did not look in until now, so a login screen
    // built on it would have listed nothing at all.
    let entry = DesktopEntry::parse(
        "hyprland",
        "[Desktop Entry]\n\
         Name=Hyprland\n\
         Comment=An intelligent dynamic tiling compositor\n\
         Exec=Hyprland\n\
         Type=Application\n",
    )
    .expect("a session entry is a desktop entry");

    assert_eq!(entry.name, "Hyprland");
    assert_eq!(
        parse_exec(&entry.exec),
        vec!["Hyprland".to_owned()],
        "and the command survives the trip, which is the half greetd needs",
    );
}

#[test]
fn sessions_are_looked_for_where_sessions_live() {
    // Both kinds, and Wayland before X: where a desktop ships both, they
    // name the same session and the Wayland one is the one to start.
    let paths = session_paths();
    let names: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .collect();
    assert!(
        names.contains(&"wayland-sessions".to_owned()),
        "wayland sessions are searched: {paths:?}",
    );
    assert!(
        names.contains(&"xsessions".to_owned()),
        "and X ones too: {paths:?}",
    );
    let wayland = names.iter().position(|name| name == "wayland-sessions");
    let x11 = names.iter().position(|name| name == "xsessions");
    assert!(wayland < x11, "wayland first: {names:?}");

    // And applications are still only looked for among applications.
    assert!(
        desktop_paths()
            .iter()
            .all(|path| path.file_name().is_some_and(|name| name == "applications")),
        "the application search did not grow session directories",
    );
}
