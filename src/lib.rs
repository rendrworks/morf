//! mold.

/// Returns this crate's display name.
pub fn name() -> &'static str {
    "mold"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_name() {
        assert_eq!(name(), "mold");
    }
}
