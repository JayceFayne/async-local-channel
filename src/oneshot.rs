use crate::{RecvError, SendError};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Poll, Waker};

#[derive(Debug)]
struct Inner<T> {
    value: Option<T>,
    waker: Option<Waker>,
    receiver: bool,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
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
        let mut inner = self.inner.borrow_mut();
        if !inner.receiver {
            return Ok(Some(value));
        }
        inner.value = Some(value);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
        Ok(None)
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Receiver<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        Rc::strong_count(&self.inner) == 1
    }

    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow_mut().value.take()
    }

    #[inline]
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            let mut inner = self.inner.borrow_mut();
            if let Some(value) = inner.value.take() {
                Poll::Ready(Ok(value))
            } else {
                if self.is_closed() {
                    Poll::Ready(Err(RecvError))
                } else {
                    if let Some(waker) = inner.waker.as_mut() {
                        waker.clone_from(cx.waker());
                    } else {
                        inner.waker = Some(cx.waker().clone());
                    }
                    Poll::Pending
                }
            }
        })
        .await
    }

    #[inline]
    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.receiver = false;
        inner.waker = None;
    }
}

#[derive(Debug)]
pub struct InactiveReceiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> InactiveReceiver<T> {
    #[inline]
    pub fn activate(self) -> Receiver<T> {
        self.inner.borrow_mut().receiver = true;
        Receiver { inner: self.inner }
    }
}

#[inline]
pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        value: None,
        waker: None,
        receiver: false,
    }));

    (
        Sender {
            inner: inner.clone(),
        },
        InactiveReceiver { inner },
    )
}

#[cfg(test)]
mod tests {
    use tokio::task::spawn_local;

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn send_before() {
        let (tx, rx) = channel();
        let rx = rx.activate();
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
        let rx = rx.activate();
        let handle = spawn_local(async move {
            assert!(rx.recv().await.unwrap());
        });
        tx.send(true).unwrap();
        handle.await.unwrap();
    }
}
