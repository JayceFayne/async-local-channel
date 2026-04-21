use crate::mpmc::{self, RecvFuture};

#[derive(Debug)]
pub struct Receiver<T> {
    inner: mpmc::Receiver<T>,
}

impl<T> Receiver<T> {
    pub fn recv(&self) -> RecvFuture<'_, T> {
        self.inner.recv()
    }

    pub fn try_recv(&self) -> Option<T> {
        self.inner.try_recv()
    }
}

pub fn channel<T>() -> (mpmc::Sender<T>, Receiver<T>) {
    let (tx, rx) = mpmc::channel();
    (tx, Receiver { inner: rx })
}
