#[repr(u8)]
pub enum NomosNodeErrorCode {
    None = 0x0,
    CouldNotInitialize = 0x1,
    StopError = 0x2,
    NullPtr = 0x3,
}
