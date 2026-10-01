use crate::{RecvError, SendError};
use alloc::rc::Rc;
use core::cell::Cell;
use core::fmt;
use core::future::poll_fn;
use core::task::{Poll, Waker};

struct Inner<T> {
    value: Cell<Option<T>>,
    waker: Cell<Option<Waker>>,
}

pub struct Sender<T> {
    inner: Rc<Inner<T>>,
}

impl<T> Sender<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        Rc::strong_count(&self.inner) == 1
    }

    #[inline]
    pub fn send(self, value: T) -> Result<Option<T>, SendError<T>> {
        if self.is_closed() {
            return Err(SendError(value));
        }
        self.inner.value.set(Some(value));
        if let Some(waker) = self.inner.waker.take() {
            waker.wake();
        }
        Ok(None)
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        if let Some(waker) = self.inner.waker.take() {
            waker.wake();
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish()
    }
}

pub struct Receiver<T> {
    inner: Rc<Inner<T>>,
}

impl<T> Receiver<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        Rc::strong_count(&self.inner) == 1
    }

    #[inline]
    pub async fn recv(self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            if let Some(value) = self.inner.value.take() {
                Poll::Ready(Ok(value))
            } else {
                if self.is_closed() {
                    Poll::Ready(Err(RecvError))
                } else {
                    let waker = if let Some(mut waker) = self.inner.waker.take() {
                        waker.clone_from(cx.waker());
                        waker
                    } else {
                        cx.waker().clone()
                    };
                    self.inner.waker.set(Some(waker));
                    Poll::Pending
                }
            }
        })
        .await
    }
}

impl<T> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.waker.take();
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

#[inline]
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(Inner {
        value: Cell::new(None),
        waker: Cell::new(None),
    });

    (
        Sender {
            inner: inner.clone(),
        },
        Receiver { inner },
    )
}

#[cfg(test)]
mod tests {
    use tokio::task::spawn_local;

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn send_before() {
        let (tx, rx) = channel();
        tx.send(true).unwrap();
        spawn_local(async move {
            assert!(rx.recv().await.unwrap());
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "local")]
    async fn send_after() {
        let (tx, rx) = channel::<bool>();
        let handle = spawn_local(async move {
            assert!(rx.recv().await.unwrap());
        });
        tx.send(true).unwrap();
        handle.await.unwrap();
    }
}
