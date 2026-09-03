//! Authenticating a person, from a configuration.
//!
//! Two shapes. `morf.pam.authenticate` is the password case: credentials go in,
//! a verdict comes back, and the shell never sees the conversation in between.
//! `morf.pam.session` is the other one -- a module asks, the configuration
//! shows the question, a person answers, the module decides. It is what a
//! fingerprint reader, a hardware key or a one-time code needs, and before it
//! existed none of those could log anybody in through morf: the engine
//! answered every prompt itself from a password it had been handed up front,
//! and dropped "touch the sensor" on the floor.

use luna::{Callback, CallbackReturn, Closure, Context, Table, UserData, UserRef};
use morf_services::{PamAuthenticator, PamSession};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::HostError, state::*};

/// How many conversations one configuration may hold open.
///
/// A lock screen has one. A greeter might have two, if it lets a fingerprint
/// and a password race. More is a configuration that forgot to cancel.
const MAX_PAM_SESSIONS: usize = 8;

/// Installs `morf.pam`.
pub(crate) fn install_pam_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let pam = Table::new(&ctx);

    // The password path: answered from what was handed in, verdict to the
    // callback. `unlock` is the one difference between the two entries -- a
    // success on the second also lifts the session lock, which is the lock
    // screen's whole reason to call it.
    for (name, unlock_on_success) in [("authenticate", false), ("authenticate_unlock", true)] {
        let pam_state = Rc::clone(&state);
        let entry = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (service, username, password, callback, confdir): (
                String,
                String,
                String,
                Closure,
                Option<String>,
            ) = stack.consume(ctx)?;
            pam_state.borrow_mut().pam_tasks.push(PendingPam {
                task: PamAuthenticator::authenticate_async(
                    service,
                    username,
                    password,
                    confdir.as_deref(),
                ),
                callback: ctx.stash(callback),
                unlock_on_success,
            });
            Ok(CallbackReturn::Return)
        });
        pam.set_field(ctx, name, entry);
    }

    // The conversation: what a session hands back and what it takes.
    let on_message_state = Rc::clone(&state);
    let session_on_message = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (session, callback): (UserRef<PamSessionToken>, Closure) = stack.consume(ctx)?;
        let mut state = on_message_state.borrow_mut();
        if state.pam_sessions.len() >= MAX_PAM_SESSIONS {
            return Err(HostError("PAM session limit reached".into()).into());
        }
        // One listener per session, replacing rather than adding: two
        // listeners answering one prompt is one answer thrown away.
        let existing = state
            .pam_sessions
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.session, &session.session));
        let entry = PendingPamSession {
            session: Rc::clone(&session.session),
            callback: ctx.stash(callback),
        };
        match existing {
            Some(index) => state.pam_sessions[index] = entry,
            None => state.pam_sessions.push(entry),
        }
        Ok(CallbackReturn::Return)
    });
    let session_respond = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (session, answer): (UserRef<PamSessionToken>, String) = stack.consume(ctx)?;
        // `false` rather than an error, because a late answer is ordinary: the
        // person pressed Enter as the module gave up, and the configuration
        // should learn that its answer went nowhere rather than crash.
        let taken = session.session.borrow_mut().respond(answer);
        stack.replace(ctx, taken);
        Ok(CallbackReturn::Return)
    });
    let session_cancel = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let session: UserRef<PamSessionToken> = stack.consume(ctx)?;
        // The listener still gets the final `finished` on the next poll; the
        // registration is dropped after that, by the drain, not here.
        session.session.borrow_mut().cancel();
        Ok(CallbackReturn::Return)
    });
    let session_methods = Table::new(&ctx);
    session_methods.set_field(ctx, "on_message", session_on_message);
    session_methods.set_field(ctx, "respond", session_respond);
    session_methods.set_field(ctx, "cancel", session_cancel);
    let session_metatable = Table::new(&ctx);
    session_metatable.set_field(ctx, "__index", session_methods);
    let session_metatable = ctx.stash(session_metatable);

    let session = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (service, username, confdir): (String, String, Option<String>) = stack.consume(ctx)?;
        let started = PamSession::start(&service, &username, confdir.as_deref());
        let userdata = UserData::new_static(
            &ctx,
            PamSessionToken {
                session: Rc::new(RefCell::new(started)),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&session_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    pam.set_field(ctx, "session", session);
    morf.set_field(ctx, "pam", pam);
}
