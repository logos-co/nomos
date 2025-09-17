pub trait TryFromRef<'a, T> {
    type Error;

    fn try_from_ref(value: &'a T) -> Result<&'a Self, Self::Error>;
}
