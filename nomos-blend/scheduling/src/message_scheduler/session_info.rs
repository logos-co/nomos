use core::{
    fmt,
    fmt::{Display, Formatter},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Session(u128);

impl From<u128> for Session {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<Session> for u128 {
    fn from(session: Session) -> Self {
        session.0
    }
}

impl Display for Session {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct SessionInfo {
    pub core_quota: usize,
    pub session_number: Session,
}
