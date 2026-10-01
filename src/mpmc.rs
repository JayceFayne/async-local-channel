use crate::{RecvError, SendError};
use alloc::collections::vec_deque::VecDeque;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use core::future::poll_fn;
use core::task::{Poll, Waker};
use slotmap::{DefaultKey, Key, SlotMap};

#[derive(Debug)]
struct State<T> {
    queue: VecDeque<T>,
    waker: SlotMap<DefaultKey, Waker>,
}

#[derive(Debug)]
struct Inner<T> {
    state: RefCell<State<T>>,
    sender: Cell<usize>,
    receiver: Cell<usize>,
}

#[derive(Debug)]
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
        if self.inner.receiver.get() == 0 {
            return Ok(Some(value));
        }
        let mut state = self.inner.state.borrow_mut();
        state.queue.push_back(value);
        for (_, waker) in state.waker.drain() {
            waker.wake();
        }
        Ok(None)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.state.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.state.borrow().queue.is_empty()
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
        if self.inner.sender.get() == 0 {
            for (_, waker) in self.inner.state.borrow_mut().waker.drain() {
                waker.wake();
            }
        }
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    inner: Rc<Inner<T>>,
    waker_key: Cell<DefaultKey>,
}

impl<T> Receiver<T> {
    fn new(inner: Rc<Inner<T>>) -> Receiver<T> {
        inner.receiver.update(|s| s + 1);
        Self {
            inner,
            waker_key: Cell::new(DefaultKey::null()),
        }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.inner.sender.get() == 0
    }

    #[inline]
    pub fn try_recv(&self) -> Option<T> {
        self.inner.state.borrow_mut().queue.pop_front()
    }

    #[inline]
    pub async fn recv(&self) -> Result<T, RecvError> {
        poll_fn(|cx| {
            let mut state = self.inner.state.borrow_mut();
            if let Some(value) = state.queue.pop_front() {
                Poll::Ready(Ok(value))
            } else {
                if self.is_closed() {
                    Poll::Ready(Err(RecvError))
                } else {
                    if let Some(waker) = state.waker.get_mut(self.waker_key.get()) {
                        waker.clone_from(cx.waker());
                    } else {
                        self.waker_key.set(state.waker.insert(cx.waker().clone()));
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
        self.inner.state.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.state.borrow().queue.is_empty()
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
        self.inner.receiver.update(|s| s - 1);
        self.inner
            .state
            .borrow_mut()
            .waker
            .remove(self.waker_key.get());
    }
}

#[derive(Debug)]
pub struct InactiveReceiver<T> {
    inner: Rc<Inner<T>>,
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
        self.inner.state.borrow().queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.state.borrow().queue.is_empty()
    }
}

#[inline]
pub fn channel<T>() -> (Sender<T>, InactiveReceiver<T>) {
    let inner = Rc::new(Inner {
        state: RefCell::new(State {
            queue: VecDeque::new(),
            waker: SlotMap::new(),
        }),
        sender: Cell::new(1),
        receiver: Cell::new(0),
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
