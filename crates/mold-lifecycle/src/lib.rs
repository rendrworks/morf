//! Delayed-destruction lifecycle state.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainState {
    pub locks: u32,
    pub dropped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetainError<T> {
    Unknown(T),
    NotLocked(T),
    TooManyLocks(T),
}

impl<T: fmt::Debug> fmt::Display for RetainError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(item) => write!(formatter, "unknown retainable {item:?}"),
            Self::NotLocked(item) => write!(formatter, "retainable {item:?} is not locked"),
            Self::TooManyLocks(item) => {
                write!(formatter, "retainable {item:?} lock count overflowed")
            }
        }
    }
}

impl<T: fmt::Debug> Error for RetainError<T> {}

#[derive(Clone, Debug)]
pub struct Retention<T> {
    entries: HashMap<T, RetainState>,
}

impl<T> Default for Retention<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T: Copy + Eq + Hash> Retention<T> {
    pub fn register(&mut self, item: T) -> bool {
        self.entries.insert(item, RetainState::default()).is_none()
    }

    pub fn unregister(&mut self, item: T) -> bool {
        self.entries.remove(&item).is_some()
    }

    pub fn state(&self, item: T) -> Option<RetainState> {
        self.entries.get(&item).copied()
    }

    pub fn lock(&mut self, item: T) -> Result<u32, RetainError<T>> {
        let state = self
            .entries
            .get_mut(&item)
            .ok_or(RetainError::Unknown(item))?;
        state.locks = state
            .locks
            .checked_add(1)
            .ok_or(RetainError::TooManyLocks(item))?;
        Ok(state.locks)
    }

    pub fn unlock(&mut self, item: T) -> Result<u32, RetainError<T>> {
        let state = self
            .entries
            .get_mut(&item)
            .ok_or(RetainError::Unknown(item))?;
        state.locks = state
            .locks
            .checked_sub(1)
            .ok_or(RetainError::NotLocked(item))?;
        Ok(state.locks)
    }

    pub fn force_unlock(&mut self, item: T) -> Result<(), RetainError<T>> {
        let state = self
            .entries
            .get_mut(&item)
            .ok_or(RetainError::Unknown(item))?;
        state.locks = 0;
        Ok(())
    }

    pub fn begin_drop(&mut self, item: T) -> Result<(), RetainError<T>> {
        self.entries
            .get_mut(&item)
            .ok_or(RetainError::Unknown(item))?
            .dropped = true;
        Ok(())
    }

    pub fn should_destroy(&self, item: T) -> Result<bool, RetainError<T>> {
        let state = self.entries.get(&item).ok_or(RetainError::Unknown(item))?;
        Ok(state.dropped && state.locks == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_delay_destruction_until_release() {
        let mut retention = Retention::default();
        assert!(retention.register(7));
        retention.lock(7).unwrap();
        retention.begin_drop(7).unwrap();
        assert!(!retention.should_destroy(7).unwrap());
        retention.unlock(7).unwrap();
        assert!(retention.should_destroy(7).unwrap());
    }

    #[test]
    fn dropped_handlers_can_acquire_a_lock() {
        let mut retention = Retention::default();
        retention.register("item");
        retention.begin_drop("item").unwrap();
        retention.lock("item").unwrap();
        assert!(!retention.should_destroy("item").unwrap());
        retention.force_unlock("item").unwrap();
        assert!(retention.should_destroy("item").unwrap());
    }
}
