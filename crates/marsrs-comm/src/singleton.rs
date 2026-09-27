//! The `OnceLock` equivalent of `mars/comm/singleton.h`.
//!
//! The C++ template offers `Create`/`Instance`/`Destroy` around a lazily
//! constructed, process-wide object with an optional destructor. Rust has no
//! need for the destructor bookkeeping — `Drop` runs when the value goes away —
//! so what is left is the lazily initialised instance itself.

use std::sync::OnceLock;

/// A lazily initialised process-wide `T`.
#[derive(Debug)]
pub struct Singleton<T> {
    cell: OnceLock<T>,
}

impl<T> Singleton<T> {
    /// An empty slot.
    pub const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    /// `Singleton::Instance()` — `None` while nothing has been created yet.
    pub fn get(&self) -> Option<&T> {
        self.cell.get()
    }

    /// `Singleton::Create()` + `Instance()` — the first caller wins, every
    /// later one gets the same value back.
    pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        self.cell.get_or_init(f)
    }

    /// `Singleton::Create()` — `false` when something was already created.
    pub fn set(&self, value: T) -> Result<(), T> {
        self.cell.set(value)
    }

    /// Requires the value to exist already; panics otherwise.
    pub fn instance(&self) -> &T {
        self.cell
            .get()
            .expect("Singleton::Instance() before Create()")
    }
}

impl<T> Default for Singleton<T> {
    fn default() -> Self {
        Self::new()
    }
}
