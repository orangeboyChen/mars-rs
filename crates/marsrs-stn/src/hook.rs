//! One callback the app is given, and the way it is called.
//!
//! What the app is handed is a callback, and what the app does with it is
//! its own business — including calling back into the port, which is how an
//! app answers one: a task it starts from the end of another one, say. A
//! callback that is run with the lock it is kept behind held is a hang the
//! moment the app does that, because a mutex taken twice on one thread never
//! lets go — and a hang is not a failure anyone sees reported.
//!
//! So [`Hook`] keeps one callback in a cell of its own, and takes it out for
//! as long as the app is being called: the call that comes back in finds
//! nothing set and goes on without it. The callback goes back when the call
//! is over, an unwind included — a callback that panicked is not one the
//! port silently loses.

use std::sync::{Arc, Mutex, PoisonError};

fn poisoned<T>(poisoned: PoisonError<T>) -> T {
    poisoned.into_inner()
}

/// One callback, behind a lock of its own and out of it while it runs.
pub struct Hook<F: ?Sized> {
    cell: Arc<Mutex<Option<Box<F>>>>,
}

impl<F: ?Sized> Hook<F> {
    /// `… = …`.
    pub fn set(&self, hook: Box<F>) {
        *self.cell.lock().unwrap_or_else(poisoned) = Some(hook);
    }

    /// Whether there is a callback in it at all.
    pub fn is_set(&self) -> bool {
        self.cell.lock().unwrap_or_else(poisoned).is_some()
    }

    /// Call the app, and put the callback back when the call is over, an
    /// unwind included. [`None`] is an unset hook, and one that is being
    /// called already.
    pub fn run<R>(&self, run: impl FnOnce(&mut F) -> R) -> Option<R> {
        let taken = self.cell.lock().unwrap_or_else(poisoned).take()?;
        let mut back = PutBack(self, Some(taken));
        Some(run(back.1.as_deref_mut()?))
    }
}

impl<F: ?Sized> Clone for Hook<F> {
    fn clone(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
        }
    }
}

impl<F: ?Sized> Default for Hook<F> {
    fn default() -> Self {
        Self {
            cell: Arc::new(Mutex::new(None)),
        }
    }
}

/// What puts a callback back into its cell when the call to the app is over.
struct PutBack<'a, F: ?Sized>(&'a Hook<F>, Option<Box<F>>);

impl<F: ?Sized> Drop for PutBack<'_, F> {
    fn drop(&mut self) {
        if let Some(hook) = self.1.take() {
            self.0.set(hook);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A hook the app answers with one more than it was asked.
    fn hook() -> Hook<dyn FnMut(u32) -> u32 + Send> {
        let hook: Hook<dyn FnMut(u32) -> u32 + Send> = Hook::default();
        hook.set(Box::new(|asked| asked + 1));
        hook
    }

    #[test]
    fn an_unset_hook_is_answered_with_nothing() {
        let hook: Hook<dyn FnMut(u32) -> u32 + Send> = Hook::default();
        assert_eq!(hook.run(|hook| hook(1)), None);
        assert!(!hook.is_set());
    }

    #[test]
    fn a_hook_is_not_held_while_the_app_is_called() {
        let cell: Hook<dyn FnMut() -> bool + Send> = Hook::default();
        let asked = cell.clone();
        cell.set(Box::new(move || {
            // What an app that answers a callback by calling back into the
            // port does: it comes back into the very hook it is in. So the
            // hook is out of its cell while the app is called, and `try_lock`
            // is how this asks.
            asked.cell.try_lock().is_ok()
        }));

        assert_eq!(
            cell.run(|hook| hook()),
            Some(true),
            "the hook is held while the app is called, so an app that calls back into the port hangs"
        );
    }

    #[test]
    fn a_call_from_inside_itself_finds_nothing_set() {
        let calls = Arc::new(AtomicUsize::new(0));
        let record = Arc::clone(&calls);
        let cell: Hook<dyn FnMut() + Send> = Hook::default();
        let again = cell.clone();
        cell.set(Box::new(move || {
            record.fetch_add(1, Ordering::SeqCst);
            // the same hook again, from inside itself: it is out, so this
            // goes on without it instead of waiting for the call it is in
            again.run(|again| again());
        }));

        cell.run(|hook| hook());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the call from inside itself ran the app twice"
        );
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn a_hook_the_app_panicked_in_is_still_there() {
        let hook: Hook<dyn FnMut(u32) -> u32 + Send> = Hook::default();
        hook.set(Box::new(|_| panic!("the app's own callback panicked")));

        let panicked =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook.run(|hook| hook(0))));
        assert!(panicked.is_err(), "the panic the app raised is caught here");
        assert!(
            hook.is_set(),
            "the callback an unwind left is still there, and a cell the panic poisoned is not one the port loses it in"
        );
    }

    #[test]
    fn the_answer_the_app_gave_is_what_the_caller_is_given() {
        assert_eq!(hook().run(|hook| hook(1)), Some(2));
        assert!(hook().is_set());
    }
}
