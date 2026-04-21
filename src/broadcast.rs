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
    listener: usize,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        let mut inner = self.inner.borrow_mut();
        if inner.listener == 0 {
            return;
        }

        inner.queue.push(value);

        for w in inner.wakers.drain(..) {
            w.wake();
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
    index: usize,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.listener -= 1;
        if inner.listener == 0 {
            inner.queue.clear();
        }
    }
}

impl<T> Receiver<T> {
    pub fn recv(&mut self) -> RecvFuture<'_, T> {
        RecvFuture { rx: self }
    }
}

pub struct RecvFuture<'a, T> {
    rx: &'a mut Receiver<T>,
}

impl<'a, T: Clone> Future for RecvFuture<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let rx = &mut self.get_mut().rx;
        let mut inner = rx.inner.borrow_mut();

        if rx.index == inner.queue.len() {
            rx.index = 0;
            inner.wakers.push(cx.waker().clone());
            if inner.listener == inner.wakers.len() {
                inner.queue.clear();
            }
            Poll::Pending
        } else {
            let value = inner.queue[rx.index].clone();
            rx.index += 1;
            Poll::Ready(value)
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
    pub fn resubscribe(&self) -> Receiver<T> {
        self.inner.borrow_mut().listener += 1;
        Receiver {
            inner: self.inner.clone(),
            index: 0,
        }
    }
}

pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        queue: Vec::new(),
        wakers: Vec::new(),
        listener: 0,
    }));

    let tx = Sender {
        inner: inner.clone(),
    };
    let rx = InactiveReceiver { inner };

    (tx, rx)
}

#[cfg(test)]
mod tests {
    use tokio::task::{JoinHandle, spawn_local};

    use super::*;

    #[tokio::test(flavor = "local")]
    async fn send_before() {
        let (tx, rx) = channel();
        for i in 0..10 {
            tx.send(Some(i));
        }

        let handle: Vec<JoinHandle<()>> = (0..10)
            .map(|_| {
                let mut rx = rx.resubscribe();
                spawn_local(async move {
                    assert!(rx.recv().await.is_none());
                })
            })
            .collect();

        tx.send(None);
        for handle in handle {
            handle.await.unwrap();
        }
    }

    #[tokio::test(flavor = "local")]
    async fn send_after() {
        let (tx, rx) = channel();

        let handle: Vec<JoinHandle<()>> = (0..10)
            .map(|_| {
                let mut rx = rx.resubscribe();
                spawn_local(async move {
                    let mut i = 0;
                    while let Some(value) = rx.recv().await {
                        i += value;
                    }
                    assert_eq!(i, 45);
                })
            })
            .collect();

        for i in 0..10 {
            tx.send(Some(i));
        }
        tx.send(None);

        for handle in handle {
            handle.await.unwrap();
        }
    }
}
