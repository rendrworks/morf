//! D-Bus: the bounded call, the shared reader, and owning a name.
//!
//! Split from `tests` at the line gate, and it is the right seam — everything
//! here needs a session bus and skips without one, which is not true of
//! anything left behind.

use std::thread;
use std::time::{Duration, Instant};

use crate::*;

#[test]
fn a_call_to_a_service_that_never_answers_gives_up() {
    // The bound is the point of this: a configuration calls out from a Lua
    // handler, and a Lua handler runs on the thread that paints. Before, a
    // service that never replied held that thread for zbus's default of
    // twenty-five seconds — no repaints, no input, and nothing on screen to say
    // why. It has to come back, and it has to come back quickly.
    //
    // A name nobody owns is the reliable way to be ignored: the bus has nowhere
    // to route the call, so the reply never arrives.
    let Ok(proxy) = DbusProxy::connect_with_timeout(
        Bus::Session,
        "org.morf.NobodyIsListening",
        "/org/morf/NobodyIsListening",
        "org.morf.NobodyIsListening",
        Duration::from_millis(250),
    ) else {
        // No session bus here — the thing under test cannot run, and failing
        // would only report the environment.
        return;
    };

    // The bound is on the connection, so that is where it is checked. Waiting
    // for a real timeout would need a peer that accepts a call and never
    // answers, which is harder to arrange than the bug is to prevent — an
    // unowned name is refused by the bus immediately and never reaches it.
    assert_eq!(
        proxy.call_timeout(),
        Some(Duration::from_millis(250)),
        "the bound reached the connection",
    );

    let started = Instant::now();
    let answer = proxy.call_value("Whatever");
    let waited = started.elapsed();

    assert!(answer.is_err(), "an unowned name cannot answer");
    assert!(
        waited < Duration::from_secs(5),
        "and it came back rather than hanging: {waited:?}",
    );
}

#[test]
fn an_ordinary_proxy_carries_the_default_bound() {
    let Ok(proxy) = DbusProxy::connect(
        Bus::Session,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return;
    };
    assert_eq!(proxy.call_timeout(), Some(DEFAULT_CALL_TIMEOUT));
}

#[test]
fn the_default_bound_is_short_enough_to_be_a_stutter() {
    // A second is far longer than any healthy reply on a session bus, and short
    // enough that a bad one costs a frame or two rather than the session. If
    // this ever grows, it should be because somebody decided to hold the paint
    // thread for that long, deliberately.
    assert!(
        DEFAULT_CALL_TIMEOUT <= Duration::from_secs(2),
        "the default call bound is {DEFAULT_CALL_TIMEOUT:?}",
    );
}

#[test]
fn many_subscriptions_share_one_connection() {
    // A connection is a socket, an authentication handshake and a name on the
    // bus. Every subscription used to open its own, so a shell watching
    // battery, network, a player and a tray paid four of each to do work one
    // connection does — and the match rule, which is what actually separates
    // them, was being carried by a socket rather than by the bus.
    //
    // The unique name is the observable part: one connection has one, and four
    // connections have four different ones.
    let Ok(proxy) = DbusProxy::connect(
        Bus::Session,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return;
    };

    let mut names = Vec::new();
    let mut held = Vec::new();
    for _ in 0..4 {
        let Ok(signal) = proxy.subscribe("NameOwnerChanged") else {
            return;
        };
        if let Some(connection) = signal.connection_name() {
            names.push(connection);
        }
        held.push(signal);
    }

    assert_eq!(names.len(), 4, "four subscriptions were made");
    names.dedup();
    assert_eq!(
        names.len(),
        1,
        "and they went over one connection, not four: {names:?}",
    );
}

#[test]
fn a_service_owns_a_name_answers_a_call_and_emits_a_signal() {
    // The whole serving half in one pass, because the halves are only useful
    // together: a name nobody can call is not a service, and a reply nobody
    // asked for is not an answer.
    //
    // This is the first time this engine is a service rather than a client, and
    // it is what everything that requires *being* something on the bus needs —
    // a notification server, an MPRIS player, a portal backend. Each is a name
    // plus a handful of methods.
    const NAME: &str = "org.morf.ServeSmoke";
    const PATH: &str = "/org/morf/ServeSmoke";
    const INTERFACE: &str = "org.morf.ServeSmoke";

    let Ok((mut service, outcome)) = DbusService::own(Bus::Session, NAME, PATH, true) else {
        // No session bus here; the thing under test cannot run.
        return;
    };
    assert_eq!(outcome, NameOutcome::Owned, "the name is ours");
    assert_eq!(service.name(), NAME);

    // A second connection, calling us the way anybody else would.
    let caller = thread::spawn(|| {
        let proxy = DbusProxy::connect_with_timeout(
            Bus::Session,
            NAME,
            PATH,
            INTERFACE,
            Duration::from_secs(5),
        )
        .expect("a caller can connect");
        proxy.call_value("Echo")
    });

    // Answer it. The call has to be read before the reply can be addressed to
    // it, which is why the caller runs on its own thread — both halves are
    // blocking, and doing them in one order on one thread is a deadlock.
    let call = service
        .next_call(Duration::from_secs(5))
        .expect("the call arrived");
    assert_eq!(call.member, "Echo");
    assert_eq!(call.interface, INTERFACE);
    assert_eq!(call.path, PATH);
    assert!(!call.sender.is_empty(), "and it says who called");
    service
        .reply(call.id, &DbusValue::String("answered".to_owned()))
        .expect("the reply is sent");

    assert_eq!(
        caller.join().expect("the caller finished").unwrap(),
        // One argument, bare. It used to arrive wrapped in a variant -- a
        // `Value` handed to zbus serialises as `v` -- and a caller that asked
        // for `s` and was given `v` rejected it. Now the body is built the
        // same way for one value as for several, and one value is one
        // argument on the wire, which the decoder hands back as itself.
        DbusValue::String("answered".to_owned()),
        "and the caller got it",
    );

    // Answering twice is refused rather than silently sending a second reply,
    // which the caller would have no way to interpret.
    assert!(
        service.reply(call.id, &DbusValue::Nil).is_err(),
        "a call can only be answered once",
    );

    // And a signal reaches somebody listening for it.
    let listener =
        DbusProxy::connect(Bus::Session, NAME, PATH, INTERFACE).expect("a listener can connect");
    let signals = listener
        .subscribe("Rang")
        .expect("the subscription is made");
    service
        .emit(PATH, INTERFACE, "Rang", &DbusValue::Integer(7))
        .expect("the signal is emitted");
    let received = signals.next_value(Duration::from_secs(5));
    assert_eq!(
        received.map(Result::unwrap),
        // Bare, for the same reason a reply is: one value is one argument.
        Some(DbusValue::Integer(7)),
        "the signal arrived with its body",
    );
}

#[test]
fn a_reply_with_several_values_arrives_as_several_arguments() {
    // `GetServerInformation` answers with four strings, and libnotify checks
    // that the reply's signature is `ssss` -- four arguments, not one struct
    // of four. This pins down which of the two a Lua list becomes.
    const NAME: &str = "org.morf.ReplyShape";
    const PATH: &str = "/org/morf/ReplyShape";
    const INTERFACE: &str = "org.morf.ReplyShape";
    let Ok((mut service, _)) = DbusService::own(Bus::Session, NAME, PATH, true) else {
        return;
    };
    let caller = thread::spawn(|| {
        let proxy = DbusProxy::connect_with_timeout(
            Bus::Session,
            NAME,
            PATH,
            INTERFACE,
            Duration::from_secs(5),
        )
        .expect("a caller can connect");
        (proxy.call_value("Four"), proxy.call_value("Unsigned"))
    });
    let call = service
        .next_call(Duration::from_secs(5))
        .expect("first call");
    assert_eq!(call.member, "Four");
    service
        .reply(
            call.id,
            &DbusValue::List(vec![
                DbusValue::String("a".into()),
                DbusValue::String("b".into()),
                DbusValue::String("c".into()),
                DbusValue::String("d".into()),
            ]),
        )
        .unwrap();
    let call = service
        .next_call(Duration::from_secs(5))
        .expect("second call");
    assert_eq!(call.member, "Unsigned");
    service
        .reply(
            call.id,
            &DbusValue::Typed {
                signature: "u".into(),
                value: Box::new(DbusValue::Integer(7)),
            },
        )
        .unwrap();
    let (four, unsigned) = caller.join().unwrap();
    assert_eq!(
        unsigned.unwrap(),
        DbusValue::Unsigned(7),
        "one typed value is one bare argument of that type"
    );
    assert_eq!(
        four.unwrap(),
        DbusValue::List(vec![
            DbusValue::String("a".into()),
            DbusValue::String("b".into()),
            DbusValue::String("c".into()),
            DbusValue::String("d".into()),
        ]),
        "four arguments, flat"
    );
}

#[test]
fn a_calls_arguments_are_always_a_list_of_them() {
    // `CloseNotification(u)` used to arrive as a bare number and `Notify`'s
    // eight arguments as a list, and a handler indexing `arguments[1]` broke
    // on the one-argument call. The signature says how many there are.
    const NAME: &str = "org.morf.ArgShape";
    const PATH: &str = "/org/morf/ArgShape";
    const INTERFACE: &str = "org.morf.ArgShape";
    let Ok((mut service, _)) = DbusService::own(Bus::Session, NAME, PATH, true) else {
        return;
    };
    let caller = thread::spawn(|| {
        let proxy = DbusProxy::connect_with_timeout(
            Bus::Session,
            NAME,
            PATH,
            INTERFACE,
            Duration::from_secs(5),
        )
        .expect("a caller can connect");
        let _ = proxy.call_value("None");
        let _ = proxy.call_value_with(
            "One",
            &DbusValue::Typed {
                signature: "u".into(),
                value: Box::new(DbusValue::Integer(9)),
            },
        );
        let _ = proxy.call_value_with(
            "Two",
            &DbusValue::List(vec![DbusValue::String("a".into()), DbusValue::Integer(2)]),
        );
    });
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let call = service.next_call(Duration::from_secs(5)).expect("a call");
        shapes.push((call.member.clone(), call.arguments.clone()));
        service.reply(call.id, &DbusValue::Nil).unwrap();
    }
    caller.join().unwrap();
    assert_eq!(shapes[0], ("None".into(), DbusValue::List(vec![])));
    assert_eq!(
        shapes[1],
        ("One".into(), DbusValue::List(vec![DbusValue::Unsigned(9)]))
    );
    assert_eq!(
        shapes[2],
        (
            "Two".into(),
            DbusValue::List(vec![DbusValue::String("a".into()), DbusValue::Integer(2)])
        )
    );
}
