use crate::{RecvError, SendError};
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::future::poll_fn;
use core::task::{Poll, Waker};
use slotmap::{DefaultKey, Key, SlotMap};

#[derive(Debug)]
struct Inner<T> {
    value: Option<T>,
    wakers: SlotMap<DefaultKey, Waker>,
    sender: usize,
    receiver: usize,
    id: usize,
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
        if inner.receiver == 0 {
            return Ok(Some(value));
        }
        inner.value = Some(value);
        inner.id = inner.id.wrapping_add(1);
        for (_, waker) in inner.wakers.drain() {
            waker.wake();
        }
        Ok(None)
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
    id: RefCell<usize>,
    waker_key: Cell<DefaultKey>,
}

impl<T> Receiver<T> {
    fn new(inner: Rc<RefCell<Inner<T>>>) -> Receiver<T> {
        inner.borrow_mut().receiver += 1;
        let id = RefCell::new(inner.borrow().id);
        Self {
            inner,
            id,
            waker_key: Cell::new(DefaultKey::null()),
        }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.borrow().sender == 0
    }

    #[inline]
    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone> Receiver<T> {
    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow().value.clone()
    }

    #[inline]
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            let mut inner = self.inner.borrow_mut();
            if *self.id.borrow() != inner.id
                && let Some(value) = inner.value.clone()
            {
                *self.id.borrow_mut() = inner.id;
                Poll::Ready(Ok(value))
            } else {
                if inner.sender == 0 {
                    Poll::Ready(Err(RecvError))
                } else {
                    if let Some(waker) = inner.wakers.get_mut(self.waker_key.get()) {
                        waker.clone_from(cx.waker());
                    } else {
                        self.waker_key.set(inner.wakers.insert(cx.waker().clone()));
                    }
                    Poll::Pending
                }
            }
        })
        .await
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
        let mut inner = self.inner.borrow_mut();
        inner.receiver -= 1;
        inner.wakers.remove(self.waker_key.get());
    }
}

pub struct RecvFuture<'a, T> {
    rx: &'a Receiver<T>,
    waker_key: Option<DefaultKey>,
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
}

#[inline]
pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        value: None,
        wakers: SlotMap::new(),
        sender: 1,
        receiver: 0,
        id: 0,
    }));

    let tx = Sender {
        inner: inner.clone(),
    };
    let rx = InactiveReceiver { inner };

    (tx, rx)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::mem;
    use tokio::task::{JoinHandle, spawn_local};

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn wait_for_change() {
        let (tx, rx) = channel();
        let rx = rx.activate();
        tx.send(1).unwrap();
        let value = rx.recv().await.unwrap();
        assert_eq!(value, 1);
        let handle = spawn_local(async move {
            tx.send(2).unwrap();
        });
        handle.await.unwrap();
        let value = rx.recv().await.unwrap();
        assert_eq!(value, 2);
    }

    #[tokio::test(flavor = "local")]
    async fn keep_last() {
        let (tx, rx) = channel();
        let rx = rx.activate();
        for i in 0..10 {
            tx.send(i).unwrap();
        }

        let value = rx.recv().await.unwrap();
        assert_eq!(value, 9);

        let handle = spawn_local(async move {
            assert_eq!(rx.recv().await.unwrap(), 10);
        });
        tx.send(10).unwrap();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "local")]
    async fn send_before() {
        let (tx, rx) = channel();
        for i in 0..10 {
            tx.send(i).unwrap();
        }

        let handle: Vec<JoinHandle<()>> = (0..10)
            .map(|_| {
                let rx = rx.clone().activate();
                spawn_local(async move {
                    assert!(rx.recv().await.is_err());
                })
            })
            .collect();
        mem::drop(tx);

        for handle in handle {
            handle.await.unwrap();
        }
    }

    #[tokio::test(flavor = "local")]
    async fn send_after() {
        let (tx, rx) = channel();

        let handle: Vec<JoinHandle<()>> = (0..10)
            .map(|_| {
                let rx = rx.clone().activate();
                spawn_local(async move {
                    assert_eq!(rx.recv().await.unwrap(), 9);
                })
            })
            .collect();

        for i in 0..10 {
            tx.send(i).unwrap();
        }

        for handle in handle {
            handle.await.unwrap();
        }
        mem::drop(tx);
    }
}
