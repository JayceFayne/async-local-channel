use crate::{RecvError, SendError};
use alloc::collections::vec_deque::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use slotmap::{DefaultKey, SlotMap};

#[derive(Debug)]
struct Inner<T> {
    queue: VecDeque<T>,
    wakers: SlotMap<DefaultKey, Waker>,
    sender: usize,
    receiver: usize,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
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

impl<T> Sender<T> {
    #[inline]
    pub fn is_closed(&self) -> bool {
        let total = Rc::strong_count(&self.inner);
        let inner = self.inner.borrow();
        inner.receiver == 0 && total != inner.receiver + inner.sender
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<Option<T>, SendError<T>> {
        let is_closed = self.is_closed();
        let mut inner = self.inner.borrow_mut();
        if inner.receiver == 0 {
            return Ok(Some(value));
        }
        if is_closed {
            return Err(SendError(value));
        }
        inner.queue.push_back(value);
        for (_, waker) in inner.wakers.drain() {
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

impl<T> Drop for Sender<T> {
    #[inline]
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sender -= 1;
        if inner.sender == 0 {
            for (_, waker) in inner.wakers.drain() {
                waker.wake();
            }
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Receiver<T> {
    fn new(inner: Rc<RefCell<Inner<T>>>) -> Receiver<T> {
        inner.borrow_mut().receiver += 1;
        Self { inner }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.borrow().sender == 0
    }

    #[inline]
    pub const fn recv(&self) -> RecvFuture<'_, T> {
        RecvFuture {
            rx: self,
            waker_key: None,
        }
    }

    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow_mut().queue.pop_front()
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

impl<T> Clone for Receiver<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self::new(self.inner.clone())
    }
}

impl<T> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        self.inner.borrow_mut().receiver -= 1;
    }
}

pub struct RecvFuture<'a, T> {
    rx: &'a Receiver<T>,
    waker_key: Option<DefaultKey>,
}

impl<'a, T> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.rx.inner.borrow_mut();
        if let Some(value) = inner.queue.pop_front() {
            Poll::Ready(Ok(value))
        } else {
            if inner.sender == 0 {
                Poll::Ready(Err(RecvError))
            } else {
                if let Some(waker_key) = self.waker_key
                    && let Some(waker) = inner.wakers.get_mut(waker_key)
                {
                    waker.clone_from(cx.waker());
                } else {
                    self.waker_key = Some(inner.wakers.insert(cx.waker().clone()));
                }
                Poll::Pending
            }
        }
    }
}

impl<T> Drop for RecvFuture<'_, T> {
    fn drop(&mut self) {
        if let Some(waker_key) = self.waker_key {
            self.rx.inner.borrow_mut().wakers.remove(waker_key);
        }
    }
}

#[derive(Debug)]
pub struct InactiveReceiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for InactiveReceiver<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> InactiveReceiver<T> {
    #[inline]
    pub fn activate(self) -> Receiver<T> {
        Receiver::new(self.inner)
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
        wakers: SlotMap::new(),
        sender: 1,
        receiver: 0,
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
