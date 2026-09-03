//! A PAM conversation, held from a configuration.
//!
//! Split from `services` at the line gate. These run libpam for real against a
//! service written into a temporary directory, so they need the dev shell's
//! own PAM on the library path -- a binary built against Nix's glibc cannot
//! reach the system's.

use std::time::Duration;

use super::*;

/// A confdir with one service whose auth step asks for a password and checks
/// it against `hunter2` through `pam_exec` -- a real module, prompting through
/// the real conversation, without needing an account on the machine.
fn pam_service_dir(name: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("morf-lua-pam-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let check = dir.join("check.sh");
    std::fs::write(&check, "#!/bin/sh\ntest \"$(cat)\" = hunter2\n").unwrap();
    std::fs::set_permissions(&check, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join(name),
        format!(
            "auth required pam_exec.so expose_authtok quiet {}\naccount required pam_permit.so\n",
            check.display()
        ),
    )
    .unwrap();
    dir
}

fn poll_until(runtime: &mut Runtime, root: morf_scene::NodeHandle, wanted: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        runtime.poll_services();
        let text = runtime
            .scene()
            .string_value(root, "text")
            .unwrap()
            .to_owned();
        if text.contains(wanted) || std::time::Instant::now() > deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_configuration_holds_a_pam_conversation() {
    // The fingerprint case, end to end from Lua: the module asks, the
    // configuration is shown the question, answers it, and is told the
    // verdict. Before this the engine answered every prompt itself from a
    // password handed in up front, so nothing a person had to take part in
    // could log anybody in.
    let dir = pam_service_dir("ask");
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pam-session.lua",
            format!(
                r#"
                    local morf = require("morf")
                    local ui = require("morf.ui")
                    local seen = morf.signal("pam.seen", "")
                    local session = morf.pam.session("ask", "nobody", "{}")
                    session:on_message(function(m)
                        seen:set(seen:get() .. m.kind .. ";")
                        if m.kind == "prompt" then
                            assert(m.echo == false, "a password prompt says it is secret")
                            assert(session:respond("hunter2"), "the prompt took the answer")
                            assert(not session:respond("again"), "and only one answer")
                        elseif m.kind == "finished" then
                            seen:set(seen:get() .. (m.ok and "ok" or ("err:" .. tostring(m.error))))
                        end
                    end)
                    ui.Text {{ text = function() return seen:get() end }}
                "#,
                dir.display()
            )
            .as_bytes(),
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = poll_until(&mut runtime, root, "finished");
    assert_eq!(
        text, "prompt;finished;ok",
        "the question reached Lua, the answer reached the module, and the verdict came back \
         (if this says libpam could not be loaded, re-enter the dev shell: the flake provides pam)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_configuration_can_give_up_on_a_pam_conversation() {
    // Nobody is going to touch the sensor. The configuration says so, and the
    // transaction ends with a verdict that says it was cancelled rather than
    // that "the conversation failed".
    let dir = pam_service_dir("cancel");
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pam-cancel.lua",
            format!(
                r#"
                    local morf = require("morf")
                    local ui = require("morf.ui")
                    local seen = morf.signal("pam.seen", "")
                    local session = morf.pam.session("cancel", "nobody", "{}")
                    session:on_message(function(m)
                        if m.kind == "prompt" then
                            session:cancel()
                            seen:set("cancelled;")
                        elseif m.kind == "finished" then
                            seen:set(seen:get() .. "finished:" .. tostring(m.code))
                        end
                    end)
                    ui.Text {{ text = function() return seen:get() end }}
                "#,
                dir.display()
            )
            .as_bytes(),
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = poll_until(&mut runtime, root, "finished");
    assert_eq!(
        text, "cancelled;finished:-1",
        "and the code names the cancel"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
