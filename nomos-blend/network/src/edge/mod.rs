pub(crate) mod handler;

// TODO: Remove test flag once the component is integrated.
#[cfg(test)]
pub use handler::ConnectionHandler;
