#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundedVec<T> {
    inner: Vec<T>,
    max_size: usize,
}

impl<T> BoundedVec<T> {
    #[must_use]
    pub const fn new(max_size: usize) -> Self {
        Self {
            inner: Vec::new(),
            max_size,
        }
    }

    pub const fn try_from_vec(vec: Vec<T>, max_size: usize) -> Result<Self, Vec<T>> {
        if vec.len() <= max_size {
            Ok(Self {
                inner: vec,
                max_size,
            })
        } else {
            Err(vec)
        }
    }

    #[must_use]
    pub fn into_inner(self) -> Vec<T> {
        self.inner
    }
}

impl<T> AsRef<[T]> for BoundedVec<T> {
    fn as_ref(&self) -> &[T] {
        self.inner.as_ref()
    }
}
