//! The conversation, exercised against real modules.
//!
//! Every test here runs libpam for real, against a service file written into
//! a temporary directory and reached through `pam_start_confdir`. That is the
//! whole point: a mock conversation would prove the mock, and what matters is
//! that a genuine module's prompt crosses the thread and a genuine answer
//! crosses back.

use super::*;
use crate::pam_conversation::PamPrompt;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A confdir with one service whose auth step asks for a password and checks
/// it against `hunter2`, through `pam_exec` -- which prompts through the
/// conversation exactly the way `pam_unix` does, without needing an account.
fn service_dir(name: &str, lines: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("morf-pam-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let check = dir.join("check.sh");
    fs::write(&check, "#!/bin/sh\ntest \"$(cat)\" = hunter2\n").unwrap();
    fs::set_permissions(&check, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.join("notice.txt"), "touch the sensor\n").unwrap();
    let service = lines
        .replace("CHECK", check.to_str().unwrap())
        .replace("NOTICE", dir.join("notice.txt").to_str().unwrap());
    fs::write(dir.join(name), service).unwrap();
    dir
}

const ASKS_FOR_A_PASSWORD: &str = "\
auth required pam_exec.so expose_authtok quiet CHECK
account required pam_permit.so
";

fn next(session: &mut PamSession) -> PamEvent {
    session
        .next(Duration::from_secs(10))
        .expect("the transaction said something within ten seconds")
}

#[test]
fn rejects_embedded_nulls_before_starting_pam() {
    let error = PamAuthenticator::authenticate("morf\0test", "user", "secret").unwrap_err();
    assert_eq!(error.code(), None);
    assert_eq!(error.to_string(), "service contains a null byte");
}

#[test]
fn asynchronous_authentication_returns_without_blocking_caller() {
    let mut task = PamAuthenticator::authenticate_async("morf\0test", "user", "secret", None);
    let error = task
        .wait(Duration::from_secs(1))
        .expect("PAM worker returned")
        .unwrap_err();
    assert_eq!(error.to_string(), "service contains a null byte");
}

#[test]
fn a_session_relays_the_prompt_and_takes_the_answer() {
    // The fingerprint case in miniature: a module asks, the shell shows the
    // question, a person answers, the module decides. Before this existed the
    // question never left the transaction thread.
    let dir = service_dir("ask", ASKS_FOR_A_PASSWORD);
    let mut session = PamSession::start("ask", "nobody", dir.to_str());

    let PamEvent::Message(PamPrompt::Prompt { text, echo }) = next(&mut session) else {
        panic!("the module's prompt reached the caller");
    };
    assert!(!echo, "a password prompt says it is secret: {text:?}");
    assert!(
        session.respond("hunter2"),
        "the prompt was open to an answer"
    );
    assert!(
        matches!(next(&mut session), PamEvent::Finished(Ok(()))),
        "and the right answer is accepted"
    );
    assert!(
        !session.respond("again"),
        "a prompt takes one answer; a second has nothing to answer"
    );

    let mut wrong = PamSession::start("ask", "nobody", dir.to_str());
    assert!(matches!(
        next(&mut wrong),
        PamEvent::Message(PamPrompt::Prompt { .. })
    ));
    wrong.respond("hunter3");
    assert!(
        matches!(next(&mut wrong), PamEvent::Finished(Err(_))),
        "and a wrong one is refused"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_message_that_wants_no_answer_still_reaches_the_caller() {
    // `pam_echo` says something and moves on, which is how a fingerprint
    // module says "touch the sensor". It used to be dropped on the floor.
    let dir = service_dir(
        "notice",
        "auth required pam_echo.so file=NOTICE\nauth required pam_permit.so\naccount required pam_permit.so\n",
    );
    let mut session = PamSession::start("notice", "nobody", dir.to_str());
    let PamEvent::Message(PamPrompt::Info(text)) = next(&mut session) else {
        panic!("the module's notice reached the caller");
    };
    assert_eq!(text.trim(), "touch the sensor");
    assert!(
        !session.respond("anything"),
        "and a notice is not open to an answer -- an answer nobody asked for is not queued"
    );
    assert!(matches!(next(&mut session), PamEvent::Finished(Ok(()))));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_ends_a_transaction_that_is_waiting_on_a_prompt() {
    // The blocked-sensor case: nobody is going to answer, and the transaction
    // must not hang the shell's shutdown waiting for them.
    let dir = service_dir("cancel", ASKS_FOR_A_PASSWORD);
    let mut session = PamSession::start("cancel", "nobody", dir.to_str());
    assert!(matches!(
        next(&mut session),
        PamEvent::Message(PamPrompt::Prompt { .. })
    ));
    session.cancel();
    let PamEvent::Finished(Err(error)) = next(&mut session) else {
        panic!("a cancelled transaction finishes, and not successfully");
    };
    assert_eq!(error.code(), Some(PAM_CANCELLED), "{error}");
    assert!(!session.respond("too late"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_password_path_answers_the_same_prompt_itself() {
    // One conversation, two ways of answering it. The fixed path drives the
    // very same module through the very same callback; only the answers come
    // from somewhere else.
    let dir = service_dir("fixed", ASKS_FOR_A_PASSWORD);
    let mut right =
        PamAuthenticator::authenticate_async("fixed", "nobody", "hunter2", dir.to_str());
    assert_eq!(right.wait(Duration::from_secs(10)), Some(Ok(())));
    let mut wrong =
        PamAuthenticator::authenticate_async("fixed", "nobody", "hunter3", dir.to_str());
    assert!(matches!(wrong.wait(Duration::from_secs(10)), Some(Err(_))));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_confdir_that_does_not_exist_is_an_error_not_a_fallback() {
    // A caller who named a directory wants to know it was not used, rather
    // than have the system's own service file silently answer instead.
    let mut session = PamSession::start("login", "nobody", Some("/nonexistent/morf-pam"));
    assert!(matches!(next(&mut session), PamEvent::Finished(Err(_))));
}
