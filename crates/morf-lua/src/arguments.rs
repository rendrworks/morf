//! What a configuration was invoked with.
//!
//! A shell is a program, and a program takes arguments. morf owns the few it
//! needs to find the configuration at all — `--clean`, `-c`, `lock` — and
//! everything after that belongs to the configuration, which is the only thing
//! that knows what its own options mean.
//!
//! Three views of the same words, because different questions want different
//! ones. [`Arguments::words`] is what was typed, in order and unaltered, for a
//! configuration that would rather read it itself. [`Arguments::options`] is the
//! flags resolved into names and values. [`Arguments::operands`] is what was
//! left over — the things that were not options.
//!
//! The grammar is the ordinary one, which is worth spelling out because
//! "ordinary" hides a lot:
//!
//! - `--name value` and `--name=value` are the same option.
//! - `--flag` with nothing after it, or followed by another option, is `true`.
//! - `-n value`, `-n=value` and `-n5` are the same option.
//! - `-abc` is `-a -b -c`, and `-abc value` gives `a` and `b` as flags with
//!   `value` on `c`.
//!
//! Those last two disagree, and something has to decide. `-n5` is a value and
//! `-ab` is two flags, but nothing here knows which options take values —
//! that is the configuration's business, and asking it would mean a schema
//! before a single argument could be read. So the rule is about the letters
//! themselves: the rest of a cluster is a *value* the moment it stops being
//! all letters. `-abc` bundles, `-n5` does not, `-ofile.txt` does not. It is
//! the same rule the eye applies, written down.
//! - `--` ends the options. Everything after it is an operand even if it starts
//!   with a dash, which is how a filename called `--help` is passed.
//! - A repeated option keeps every value, because `--font a --font b` meaning
//!   only `b` is a guess about intent that nothing here is in a position to
//!   make.

use std::collections::BTreeMap;
use std::sync::OnceLock;

static GIVEN: OnceLock<Arguments> = OnceLock::new();

/// Records what this process was invoked with, once.
///
/// Process-wide because a command line is: every configuration this process
/// loads was started by the same words, and threading them down five function
/// signatures would only be describing that fact at greater length.
pub fn install(words: Vec<String>) {
    let _ = GIVEN.set(Arguments::parse(words));
}

/// What this process was invoked with, or nothing if it was never told.
pub fn given() -> &'static Arguments {
    static NONE: OnceLock<Arguments> = OnceLock::new();
    GIVEN
        .get()
        .unwrap_or_else(|| NONE.get_or_init(Arguments::default))
}

/// One option's values, in the order they were given.
pub type Values = Vec<Value>;

/// What an option carried: a bare flag, or something after it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Present, with nothing attached.
    Flag,
    Text(String),
}

impl Value {
    /// The text of a value, or nothing for a bare flag.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Flag => None,
            Self::Text(text) => Some(text),
        }
    }
}

/// A configuration's arguments, in the three shapes worth having.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arguments {
    words: Vec<String>,
    options: BTreeMap<String, Values>,
    operands: Vec<String>,
}

impl Arguments {
    /// Reads a command line the way a command line is ordinarily read.
    pub fn parse(words: Vec<String>) -> Self {
        let mut options: BTreeMap<String, Values> = BTreeMap::new();
        let mut operands = Vec::new();
        let mut rest = words.iter().peekable();
        let mut only_operands = false;

        // Whether the word after this one is this option's value, or the next
        // thing entirely. It is a value unless it looks like an option itself —
        // which is why `--verbose --output x` reads as a flag and an option
        // rather than `verbose` taking `--output`.
        let takes_next = |next: Option<&&String>| {
            next.is_some_and(|word| *word == "-" || !word.starts_with('-'))
        };

        while let Some(word) = rest.next() {
            if only_operands {
                operands.push(word.clone());
            } else if word == "--" {
                only_operands = true;
            } else if let Some(body) = word.strip_prefix("--") {
                let (name, inline) = split_once(body);
                let value = match inline {
                    Some(text) => Value::Text(text),
                    None if takes_next(rest.peek()) => {
                        Value::Text(rest.next().expect("peeked").clone())
                    }
                    None => Value::Flag,
                };
                options.entry(name).or_default().push(value);
            } else if word.len() > 1
                && let Some(body) = word.strip_prefix('-')
            {
                let letters: Vec<char> = body.chars().collect();
                for (index, letter) in letters.iter().enumerate() {
                    let last = index + 1 == letters.len();
                    let tail: String = letters[index + 1..].iter().collect();
                    let tail_is_letters =
                        !tail.is_empty() && tail.chars().all(|character| character.is_alphabetic());
                    let value = if let Some(inline) = tail.strip_prefix('=') {
                        Some(Value::Text(inline.to_owned()))
                    } else if !tail.is_empty() && !tail_is_letters {
                        // The cluster stopped being letters, so the rest is
                        // what this one carries: `-n5`, `-ofile.txt`.
                        Some(Value::Text(tail))
                    } else if last && takes_next(rest.peek()) {
                        Some(Value::Text(rest.next().expect("peeked").clone()))
                    } else {
                        None
                    };
                    let attached = value.is_some();
                    options
                        .entry(letter.to_string())
                        .or_default()
                        .push(value.unwrap_or(Value::Flag));
                    if attached {
                        break;
                    }
                }
            } else {
                operands.push(word.clone());
            }
        }

        Self {
            words,
            options,
            operands,
        }
    }

    /// Everything that was typed, in order and unaltered.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// The options, by name.
    pub fn options(&self) -> &BTreeMap<String, Values> {
        &self.options
    }

    /// What was not an option.
    pub fn operands(&self) -> &[String] {
        &self.operands
    }
}

/// `name=value`, where the value may be empty and the name may not contain the
/// separator. Split at the first `=`, so `--filter=a=b` filters on `a=b`.
fn split_once(body: &str) -> (String, Option<String>) {
    match body.split_once('=') {
        Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
        None => (body.to_owned(), None),
    }
}
