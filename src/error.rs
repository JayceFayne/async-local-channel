use core::error::Error;
use core::fmt::{Debug, Display, Formatter, Result};

pub struct SendError<T>(pub T);

impl<T> Debug for SendError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "channel closed")
    }
}

impl<T> Display for SendError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "channel closed")
    }
}

impl<T> Error for SendError<T> {}

#[derive(Debug)]
pub struct RecvError;

impl Display for RecvError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "channel closed")
    }
}

impl Error for RecvError {}
