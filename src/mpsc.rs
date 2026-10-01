use crate::{RecvError, SendError};
use alloc::collections::vec_deque::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Poll, Waker};

#[derive(Debug)]
struct Inner<T> {
    queue: VecDeque<T>,
    waker: Option<Waker>,
    sender: usize,
    receiver: bool,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Sender<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        Rc::strong_count(&self.inner) == self.inner.borrow().sender
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<Option<T>, SendError<T>> {
        if self.is_closed() {
            return Err(SendError(value));
        }
        let mut inner = self.inner.borrow_mut();
        if !inner.receiver {
            return Ok(Some(value));
        }
        inner.queue.push_back(value);
        if let Some(waker) = inner.waker.take() {
            waker.wake();
        }
        Ok(None)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().queue.is_empty()
    }
}

impl<T> Clone for Sender<T> {
    #[inline]
    fn clone(&self) -> Self {
        self.inner.borrow_mut().sender += 1;
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sender -= 1;
        if inner.sender == 0
            && let Some(waker) = inner.waker.take()
        {
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
        self.inner.borrow().sender == 0
    }

    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow_mut().queue.pop_front()
    }

    #[inline]
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            let mut inner = self.inner.borrow_mut();
            if let Some(value) = inner.queue.pop_front() {
                Poll::Ready(Ok(value))
            } else {
                if inner.sender == 0 {
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

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().queue.is_empty()
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

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().queue.is_empty()
    }
}

#[inline]
pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        queue: VecDeque::new(),
        waker: None,
        sender: 1,
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
