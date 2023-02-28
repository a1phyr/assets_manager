use crate::{AnyCache, BoxedError, Compound, SharedString};
use once_cell::sync::OnceCell;
use std::{cell::UnsafeCell, fmt, mem::ManuallyDrop};

union State<U, T> {
    uninit: ManuallyDrop<U>,
    init: ManuallyDrop<T>,
}

/// A thread-safe cell which can be written to only once.
///
/// This is just like a [`OnceCell`], but it also has data when uninitialized.
///
/// This is useful if an asset needs a context to be fully initialized. The
/// "raw" version of the asset can be stored as the `U`ninitialized part of
/// the cell, and further loading can be done later when additional context
/// is available.
///
/// The type also provides easy integration with hot-reloading: when the
/// "uninitialized" value is reloaded, so is the cell, and the
/// initialization is re-run.
///
/// # Example
///
/// ```no_run
/// use assets_manager::{asset::Png, AnyCache, BoxedError, OnceInitCell};

/// struct GpuCtx(/* ... */);
/// struct Texture(/* ... */);
///
/// impl GpuCtx {
///     /// Loads a texture to GPU from an image
///     fn load_texture(&self, img: &image::DynamicImage) -> Texture {
///         /* ... */
///         # todo!()
///     }
///
///     /// Does something with a GPU texture
///     fn use_texture(&self, texture: &Texture) {
///         /* ... */
///         # todo!()
///     }
///
///     /// Loads a texture from an image the cache and uses it.
///     fn load_and_use_texture(&self, cache: AnyCache, id: &str) -> Result<(), BoxedError> {
///         // Load the cached texture or the source PNG image.
///         let img = cache.load::<OnceInitCell<Png, Texture>>(id)?.read();
///
///         // If the image has not been uploaded to GPU yet or if it has been
///         // reloaded, upload it.
///         let texture = img.get_or_init(|img| self.load_texture(&img.0));
///
///         self.use_texture(texture);
///
///         Ok(())
///     }
/// }
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "utils")))]
pub struct OnceInitCell<U, T> {
    once: OnceCell<()>,
    // Safety:
    // - Shared access to `data.init` field if `once` is initialized
    // - Mutable access to `data.uninit` within `once` initializer
    data: UnsafeCell<State<U, T>>,
}

// We don't need `U: Sync` because it is only accessed through a `&mut`
unsafe impl<U, T> Sync for OnceInitCell<U, T>
where
    T: Send + Sync,
    U: Send,
{
}

impl<U, T> OnceInitCell<U, T> {
    /// Creates a new uninitialized cell.
    #[inline]
    pub const fn new(value: U) -> Self {
        Self {
            once: OnceCell::new(),
            data: UnsafeCell::new(State {
                uninit: ManuallyDrop::new(value),
            }),
        }
    }

    /// Creates a new initialized cell.
    #[inline]
    pub const fn with_value(value: T) -> Self {
        Self {
            once: OnceCell::with_value(()),
            data: UnsafeCell::new(State {
                init: ManuallyDrop::new(value),
            }),
        }
    }

    #[inline]
    unsafe fn get_unchecked(&self) -> &T {
        &(*self.data.get()).init
    }

    /// Gets the reference to the underlying value.
    ///
    /// Returns `None` if the cell is empty, or being initialized. This
    /// method never blocks.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        match self.once.get() {
            Some(_) => unsafe { Some(self.get_unchecked()) },
            None => None,
        }
    }

    /// Gets the contents of the cell, initializing it with `f` if the cell
    /// was uninitialized.
    ///
    /// See `get_or_try_init` for more details.
    #[inline]
    pub fn get_or_init(&self, f: impl FnOnce(&mut U) -> T) -> &T {
        match self.get_or_try_init(|u| Ok::<_, std::convert::Infallible>(f(u))) {
            Ok(v) => v,
            Err(never) => match never {},
        }
    }

    /// Gets the contents of the cell, initializing it with `f` if the cell
    /// was uninitialized. If the cell was uninitialized and `f` failed, an
    /// error is returned.
    ///
    /// # Panics
    ///
    /// If `f` panics, the panic is propagated to the caller, and the cell
    /// remains uninitialized.
    ///
    /// It is an error to reentrantly initialize the cell from `f`. The
    /// exact outcome is unspecified.
    pub fn get_or_try_init<E>(&self, f: impl FnOnce(&mut U) -> Result<T, E>) -> Result<&T, E> {
        unsafe {
            let mut uninit_value = None;

            self.once.get_or_try_init(|| {
                // Safety: synchronisation through the `OnceCell`
                let state = &mut *self.data.get();

                let value = f(&mut state.uninit)?;

                let new_state = State {
                    init: ManuallyDrop::new(value),
                };
                // We don't drop the unitialized value within the closure to
                // avoid being in a bad state if the `Drop` impl panics.
                //
                // By making the value "escape" the closure, we make sure
                // that the closure always returns if `f` returns.
                let uninit = std::mem::replace(state, new_state).uninit;
                uninit_value = Some(ManuallyDrop::into_inner(uninit));
                Ok(())
            })?;

            // This would be done automatically but we don't want it in the
            // happy path.
            if let Some(value) = uninit_value {
                drop_cold(value);
            }

            Ok(self.get_unchecked())
        }
    }
}

impl<U, T> Drop for OnceInitCell<U, T> {
    fn drop(&mut self) {
        unsafe {
            let data = self.data.get_mut();
            match self.once.get_mut() {
                Some(_) => ManuallyDrop::drop(&mut data.init),
                None => ManuallyDrop::drop(&mut data.uninit),
            }
        }
    }
}

impl<U, T> Default for OnceInitCell<U, T>
where
    U: Default,
{
    fn default() -> Self {
        Self::new(U::default())
    }
}

impl<U, T: fmt::Debug> fmt::Debug for OnceInitCell<U, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get() {
            Some(data) => f.debug_tuple("MyCell").field(data).finish(),
            None => f.write_str("MyCell(<uninit>)"),
        }
    }
}

impl<U: Compound, T: Send + Sync + 'static> Compound for OnceInitCell<U, T> {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        Ok(OnceInitCell::new(cache.load_owned(id)?))
    }

    const HOT_RELOADED: bool = U::HOT_RELOADED;
}

impl<U: Compound, T: Send + Sync + 'static> Compound for OnceInitCell<Option<U>, T> {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        Ok(OnceInitCell::new(Some(cache.load_owned(id)?)))
    }

    const HOT_RELOADED: bool = U::HOT_RELOADED;
}

/// Like `drop` but cold to keep this out of the happy path
#[cold]
fn drop_cold<T>(_x: T) {}
