use crate::{RecvError, SendError};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

#[derive(Debug)]
struct Inner<T> {
    value: Option<T>,
    wakers: Vec<Waker>,
    sender: usize,
    receiver: usize,
    id: usize,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.inner.borrow_mut().sender += 1;
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sender -= 1;
        if inner.sender == 0 {
            for waker in inner.wakers.drain(..) {
                waker.wake();
            }
        }
    }
}

impl<T> Sender<T> {
    pub fn is_closed(&self) -> bool {
        let total = Rc::strong_count(&self.inner);
        let inner = self.inner.borrow();
        inner.receiver == 0 && total != inner.receiver + inner.sender
    }

    pub fn send(&self, value: T) -> Result<Option<usize>, SendError<T>> {
        let is_closed = self.is_closed();
        let mut inner = self.inner.borrow_mut();
        if inner.receiver == 0 {
            return Ok(None);
        }
        if is_closed {
            return Err(SendError(value));
        }
        inner.value = Some(value);
        inner.id = inner.id.wrapping_add(1);
        let woken = inner.wakers.len();
        for waker in inner.wakers.drain(..) {
            waker.wake();
        }
        Ok(Some(woken))
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
    id: RefCell<usize>,
}

impl<T> Receiver<T> {
    fn new(inner: Rc<RefCell<Inner<T>>>) -> Receiver<T> {
        inner.borrow_mut().receiver += 1;
        let id = RefCell::new(inner.borrow().id);
        Self { inner, id }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.borrow().sender == 0
    }

    pub fn recv(&self) -> RecvFuture<'_, T> {
        RecvFuture { rx: self }
    }

    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone> Receiver<T> {
    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow().value.clone()
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self::new(self.inner.clone())
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.borrow_mut().receiver -= 1;
    }
}

pub struct RecvFuture<'a, T> {
    rx: &'a Receiver<T>,
}

impl<'a, T: Clone> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rx = &mut self.get_mut().rx;
        let mut inner = rx.inner.borrow_mut();
        if let Some(value) = inner.value.clone()
            && *rx.id.borrow() != inner.id
        {
            *rx.id.borrow_mut() = inner.id;
            Poll::Ready(Ok(value))
        } else {
            if inner.sender == 0 {
                Poll::Ready(Err(RecvError))
            } else {
                inner.wakers.push(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[derive(Debug)]
pub struct InactiveReceiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for InactiveReceiver<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> InactiveReceiver<T> {
    pub fn activate(self) -> Receiver<T> {
        Receiver::new(self.inner)
    }
}

pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        value: None,
        wakers: Vec::new(),
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
