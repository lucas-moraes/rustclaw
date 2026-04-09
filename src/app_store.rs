use std::sync::Arc;
use std::sync::RwLock;

type StateChangeCallback<T> = Arc<dyn Fn(&T, &T) + Send + Sync>;

pub struct Store<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    state: RwLock<T>,
    subscribers: Arc<RwLock<Vec<StateChangeCallback<T>>>>,
}

impl<T> Store<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    pub fn new(initial_state: T) -> Self {
        Self {
            state: RwLock::new(initial_state),
            subscribers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn get_state(&self) -> T {
        self.state.read().expect("app state lock poisoned").clone()
    }

    pub fn set_state<F>(&self, updater: F)
    where
        F: FnOnce(&T) -> T,
    {
        let old = self.get_state();
        let new = updater(&old);

        if new != old {
            *self.state.write().expect("app state lock poisoned") = new.clone();
            self.notify_subscribers(&old, &new);
        }
    }

    pub fn subscribe<F>(&self, callback: F) -> impl FnMut() + Send + Sync
    where
        F: Fn(&T, &T) + Send + Sync + 'static,
    {
        let subs = self.subscribers.clone();
        let wrapper: StateChangeCallback<T> = Arc::new(callback);
        let wrapper_for_closure = wrapper.clone();
        subs.write()
            .expect("subscribers lock poisoned")
            .push(wrapper);

        move || {
            subs.write()
                .expect("subscribers lock poisoned")
                .retain(|s| !Arc::ptr_eq(s, &wrapper_for_closure));
        }
    }

    fn notify_subscribers(&self, old_state: &T, new_state: &T) {
        let subs = self.subscribers.read().expect("subscribers lock poisoned");
        for sub in subs.iter() {
            sub(new_state, old_state);
        }
    }
}

impl<T> Default for Store<T>
where
    T: Clone + PartialEq + Default + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}
