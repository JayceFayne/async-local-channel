use crate::{RecvError, SendError};
use alloc::collections::vec_deque::VecDeque;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

#[derive(Debug)]
struct Inner<T> {
    queue: VecDeque<T>,
    waker: Option<Waker>,
    sender: usize,
    receiver: u8,
}

#[derive(Debug)]
pub struct Sender<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Sender<T> {
    pub fn is_closed(&self) -> bool {
        let total = Rc::strong_count(&self.inner);
        let inner = self.inner.borrow();
        inner.receiver as usize == 0 && total != inner.receiver as usize + inner.sender
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
        inner.queue.push_back(value);
        Ok(Some(if let Some(waker) = inner.waker.take() {
            waker.wake();
            inner.receiver as usize
        } else {
            0
        }))
    }
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
        if let Some(waker) = inner.waker.take()
            && inner.sender == 0
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
    pub fn is_closed(&self) -> bool {
        self.inner.borrow().sender == 0
    }

    pub fn recv(&self) -> RecvFuture<'_, T> {
        RecvFuture { rx: self }
    }

    pub fn try_recv(&self) -> Option<T> {
        self.inner.borrow_mut().queue.pop_front()
    }

    pub fn deactivate(self) -> InactiveReceiver<T> {
        InactiveReceiver {
            inner: self.inner.clone(),
        }
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

impl<'a, T> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.rx.inner.borrow_mut();
        if let Some(value) = inner.queue.pop_front() {
            Poll::Ready(Ok(value))
        } else {
            if inner.sender == 0 {
                Poll::Ready(Err(RecvError))
            } else {
                inner.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[derive(Debug)]
pub struct InactiveReceiver<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> InactiveReceiver<T> {
    pub fn activate(self) -> Receiver<T> {
        self.inner.borrow_mut().receiver += 1;
        Receiver { inner: self.inner }
    }
}

pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(RefCell::new(Inner {
        queue: VecDeque::new(),
        waker: None,
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
