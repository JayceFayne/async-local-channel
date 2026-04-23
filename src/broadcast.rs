use crate::{RecvError, SendError};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

#[derive(Debug)]
struct Inner<T> {
    queue: Vec<T>,
    wakers: Vec<Waker>,
    sender: usize,
    receiver: usize,
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
        inner.queue.push(value);
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
    index: usize,
}

impl<T> Receiver<T> {
    fn new(inner: Rc<RefCell<Inner<T>>>, index: usize) -> Receiver<T> {
        inner.borrow_mut().receiver += 1;
        Self { inner, index }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.borrow().sender == 0
    }

    pub fn recv(&mut self) -> RecvFuture<'_, T> {
        RecvFuture { rx: self }
    }

    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Clone> Receiver<T> {
    pub fn try_recv(&mut self) -> Option<T> {
        let value = self.inner.borrow_mut().queue.get(self.index)?.clone();
        self.index += 1;
        Some(value)
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self::new(self.inner.clone(), self.index)
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.borrow_mut().receiver -= 1;
    }
}

pub struct RecvFuture<'a, T> {
    rx: &'a mut Receiver<T>,
}

impl<'a, T: Clone> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rx = &mut self.get_mut().rx;
        let mut inner = rx.inner.borrow_mut();
        if rx.index == inner.queue.len() {
            rx.index = 0; //FIXME: 
            if inner.sender == 0 {
                Poll::Ready(Err(RecvError))
            } else {
                inner.wakers.push(cx.waker().clone());
                if inner.receiver == inner.wakers.len() {
                    inner.queue.clear();
                }
                Poll::Pending
            }
        } else {
            let value = inner.queue[rx.index].clone();
            rx.index += 1;
            Poll::Ready(Ok(value))
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
        Receiver::new(self.inner, 0)
    }
}

pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        queue: Vec::new(),
        wakers: Vec::new(),
        sender: 1,
        receiver: 0,
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
    async fn send_before() {
        let (tx, rx) = channel();
        for i in 0..10 {
            tx.send(i).unwrap();
        }

        let handle: Vec<JoinHandle<()>> = (0..10)
            .map(|_| {
                let mut rx = rx.clone().activate();
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
                let mut rx = rx.clone().activate();
                spawn_local(async move {
                    let mut i = 0;
                    while let Ok(value) = rx.recv().await {
                        i += value;
                    }
                    assert_eq!(i, 45);
                })
            })
            .collect();

        for i in 0..10 {
            tx.send(i).unwrap();
        }
        mem::drop(tx);

        for handle in handle {
            handle.await.unwrap();
        }
    }
}
