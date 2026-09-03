//! The argument grammar, which is ordinary — and "ordinary" hides a lot.

use crate::arguments::{Arguments, Value};

fn parse(words: &[&str]) -> Arguments {
    Arguments::parse(words.iter().map(|word| (*word).to_owned()).collect())
}

fn text(arguments: &Arguments, name: &str) -> Vec<Option<String>> {
    arguments
        .options()
        .get(name)
        .map(|values| {
            values
                .iter()
                .map(|value| value.text().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_long_option_takes_its_value_either_way_round() {
    for words in [["--user", "trim"], ["--user=trim", "--"]] {
        let parsed = parse(&words);
        assert_eq!(text(&parsed, "user"), [Some("trim".to_owned())]);
    }
}

/// A flag followed by another option is a flag, not an option whose value is
/// the next option. `--verbose --output x` is two options and always was.
#[test]
fn a_flag_does_not_swallow_the_option_after_it() {
    let parsed = parse(&["--verbose", "--output", "x"]);
    assert_eq!(parsed.options().get("verbose"), Some(&vec![Value::Flag]));
    assert_eq!(text(&parsed, "output"), [Some("x".to_owned())]);
}

/// `-abc` is three flags, and `-abc value` is two flags and one option — the
/// value belongs to the last letter, which is the only one that can take it.
#[test]
fn short_options_bundle_and_the_last_one_takes_the_value() {
    let bare = parse(&["-abc"]);
    for letter in ["a", "b", "c"] {
        assert_eq!(bare.options().get(letter), Some(&vec![Value::Flag]));
    }
    //  is one option carrying a path, not seven flags.
    let attached = parse(&["-ofile.txt"]);
    assert_eq!(text(&attached, "o"), [Some("file.txt".to_owned())]);
    // A path attached to its letter is one option, not seven flags.
    let attached = parse(&["-ofile.txt"]);
    assert_eq!(text(&attached, "o"), [Some("file.txt".to_owned())]);
    let carried = parse(&["-abc", "file"]);
    assert_eq!(carried.options().get("a"), Some(&vec![Value::Flag]));
    assert_eq!(carried.options().get("b"), Some(&vec![Value::Flag]));
    assert_eq!(text(&carried, "c"), [Some("file".to_owned())]);
    assert!(carried.operands().is_empty(), "the value is not an operand");
}

/// And where bundling and attachment disagree, the letters decide: a cluster
/// stops being flags the moment it stops being letters.
#[test]
fn a_short_option_takes_its_value_attached_or_apart() {
    for words in [["-n", "5"], ["-n5", "--"], ["-n=5", "--"]] {
        let parsed = parse(&words);
        assert_eq!(text(&parsed, "n"), [Some("5".to_owned())], "{words:?}");
    }
}

/// After `--` nothing is an option, which is the only way to pass a filename
/// that begins with a dash.
#[test]
fn the_separator_ends_the_options() {
    let parsed = parse(&["--real", "--", "--help", "-x"]);
    assert_eq!(parsed.options().get("real"), Some(&vec![Value::Flag]));
    assert_eq!(parsed.operands(), ["--help", "-x"]);
    assert!(parsed.options().get("help").is_none());
}

/// A repeated option keeps every value. Which one wins is the configuration's
/// business — deciding here would be guessing at an intent this cannot see.
#[test]
fn a_repeated_option_keeps_what_it_was_given() {
    let parsed = parse(&["--font", "a", "--font", "b"]);
    assert_eq!(
        text(&parsed, "font"),
        [Some("a".to_owned()), Some("b".to_owned())]
    );
}

/// A bare `-` is a name for standard input by long convention, not an option.
#[test]
fn a_lone_dash_is_an_operand() {
    let parsed = parse(&["cat", "-"]);
    assert_eq!(parsed.operands(), ["cat", "-"]);
    assert!(parsed.options().is_empty());
}

/// The first `=` separates, so a value may contain as many more as it likes.
#[test]
fn only_the_first_separator_separates() {
    let parsed = parse(&["--filter=a=b"]);
    assert_eq!(text(&parsed, "filter"), [Some("a=b".to_owned())]);
}

#[test]
fn what_was_typed_is_kept_exactly() {
    let words = ["--user=trim", "--", "-x"];
    assert_eq!(parse(&words).words(), words);
}
