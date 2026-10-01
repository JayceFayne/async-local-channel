use crate::{RecvError, SendError};
use alloc::collections::vec_deque::VecDeque;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::fmt;
use core::future::poll_fn;
use core::task::{Poll, Waker};

struct Inner<T> {
    queue: RefCell<VecDeque<T>>,
    waker: Cell<Option<Waker>>,
    sender: Cell<usize>,
    receiver: Cell<bool>,
}

pub struct Sender<T> {
    inner: Rc<Inner<T>>,
}

impl<T> Sender<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        Rc::strong_count(&self.inner) == self.inner.sender.get()
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<Option<T>, SendError<T>> {
        if self.is_closed() {
            return Err(SendError(value));
        }
        if !self.inner.receiver.get() {
            return Ok(Some(value));
        }
        self.inner.queue.borrow_mut().push_back(value);
        if let Some(waker) = self.inner.waker.take() {
            waker.wake();
        }
        Ok(None)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.queue.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.queue.borrow().is_empty()
    }
}

impl<T> Clone for Sender<T> {
    #[inline]
    fn clone(&self) -> Self {
        self.inner.sender.update(|s| s + 1);
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.sender.update(|s| s - 1);
        if self.inner.sender.get() == 0
            && let Some(waker) = self.inner.waker.take()
        {
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
        self.inner.sender.get() == 0
    }

    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.queue.borrow_mut().pop_front()
    }

    #[inline]
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            if let Some(value) = self.inner.queue.borrow_mut().pop_front() {
                Poll::Ready(Ok(value))
            } else {
                if self.inner.sender.get() == 0 {
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

    #[inline]
    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.queue.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.queue.borrow().is_empty()
    }
}
impl<T> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.receiver.set(false);
        self.inner.waker.take();
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

pub struct InactiveReceiver<T> {
    inner: Rc<Inner<T>>,
}

impl<T> InactiveReceiver<T> {
    #[inline]
    pub fn activate(self) -> Receiver<T> {
        self.inner.receiver.set(true);
        Receiver { inner: self.inner }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.queue.borrow().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.queue.borrow().is_empty()
    }
}

impl<T> fmt::Debug for InactiveReceiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InactiveReceiver").finish()
    }
}

#[inline]
pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(Inner {
        queue: RefCell::new(VecDeque::new()),
        waker: Cell::new(None),
        sender: Cell::new(1),
        receiver: Cell::new(false),
    });

    (
        Sender {
            inner: inner.clone(),
        },
        InactiveReceiver { inner },
    )
}

#[cfg(test)]
mod tests {
    use core::mem;

    use tokio::task::spawn_local;

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn send_before() {
        let (tx, rx) = channel();
        let rx = rx.activate();
        for i in 0..10 {
            tx.send(i).unwrap();
        }
        mem::drop(tx);
        spawn_local(async move {
            let mut i = 0;
            while let Ok(value) = rx.recv().await {
                i += value;
            }
            assert_eq!(i, 45);
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "local")]
    async fn send_after() {
        let (tx, rx) = channel();
        let rx = rx.activate();
        let handle = spawn_local(async move {
            let mut i = 0;
            while let Ok(value) = rx.recv().await {
                i += value;
            }
            assert_eq!(i, 45);
        });
        for i in 0..10 {
            tx.send(i).unwrap();
        }
        mem::drop(tx);
        handle.await.unwrap();
    }
}
