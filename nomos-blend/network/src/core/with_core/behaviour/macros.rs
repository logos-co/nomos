/// Creates a [`CurrentSessionBehaviour`] from the references of [`Behaviour`] and [`Session`].
///
/// It is defined as a macro instead of a method of [`Behaviour`]
/// because the macro should accept both mutable references
/// that point to the same [`Behaviour`].
#[macro_export]
macro_rules! current_session_behaviour {
    ($behaviour:expr, $session:expr) => {
        CurrentSessionBehaviour {
            common: CommonSessionBehaviour {
                session: $session,
                events: &mut $behaviour.events,
                waker: &mut $behaviour.waker,
            },
            local_peer_id: $behaviour.local_peer_id,
        }
    };
}

/// Creates a [`PreviousSessionBehaviour`] from the references of [`Behaviour`] and [`Session`].
///
/// It is defined as a macro instead of a method of [`Behaviour`]
/// because the macro should accept both mutable references
/// that point to the same [`Behaviour`].
#[macro_export]
macro_rules! previous_session_behaviour {
    ($behaviour:expr, $session:expr) => {
        PreviousSessionBehaviour(CommonSessionBehaviour {
            session: $session,
            events: &mut $behaviour.events,
            waker: &mut $behaviour.waker,
        })
    };
}
