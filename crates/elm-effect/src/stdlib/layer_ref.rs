//! `LayerRef` port (`LayerRef.ts` at the pin): one memoized build.
//!
//! A single memoized value with explicit [`refresh`](LayerRef::refresh),
//! over a plain constructor closure (see [`LayerMap`](super::layer_map::LayerMap)
//! for why layers reduce to constructors here).
//!
//! Residuals: `Service` tag derivation and layer-scoped lifetimes.

/// One memoized build with explicit refresh.
pub struct LayerRef<V> {
    build: Box<dyn Fn() -> V + Send + Sync>,
    current: std::sync::Mutex<Option<V>>,
}

impl<V> LayerRef<V> {
    /// Build from a constructor (`make`).
    pub fn make(build: impl Fn() -> V + Send + Sync + 'static) -> Self {
        Self {
            build: Box::new(build),
            current: std::sync::Mutex::new(None),
        }
    }

    /// Memoized value, building once.
    pub fn get(&self) -> V
    where
        V: Clone,
    {
        let mut guard = self.current.lock().expect("layer ref");
        if guard.is_none() {
            *guard = Some((self.build)());
        }
        guard.as_ref().expect("just built").clone()
    }

    /// Forget the memoized value so the next [`get`](LayerRef::get) rebuilds.
    pub fn refresh(&self) {
        *self.current.lock().expect("layer ref") = None;
    }

    /// `true` while a value is memoized.
    pub fn is_built(&self) -> bool {
        self.current.lock().expect("layer ref").is_some()
    }
}
