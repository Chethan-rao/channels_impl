use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

// Sender
struct Sender<T> {
    shared: Arc<Shared<T>>,
}

// We need manual implementation of Clone for Sender, because the derive[Clone] generates a Clone impl in which it adds a Clone trait bound to T which is not what we want. We just want to clone the inner Arc value and not the T. Hence we need manual Clone impl
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.senders_count += 1;

        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.senders_count -= 1;
        if inner.senders_count == 0 {
            // If all senders goes out of scope, we need to tell receiver to wake up
            self.shared.available.notify_one();
        }
    }
}

impl<T> Sender<T> {
    fn send(&self, data: T) -> Result<(), &'static str> {
        let mut channel = self.shared.inner.lock().unwrap();

        if !channel.is_channel_still_active {
            return Err("Channel is closed");
        };

        channel.queue.push_back(data);
        drop(channel); // this guard would be dropped nontheless after this function ends. but its better to drop the guard before calling the notify so that other sleeping thread can immediatly take the guard
        self.shared.available.notify_one();
        Ok(())
    }
}

// Reciever
struct Reciever<T> {
    shared: Arc<Shared<T>>,
    // Since we know that this is the only reciever that's going to consume the data from sender, instead of attaining lock on each recv() call, maintain a buffer that stores the current elements in the queue by popping everything from queue during first recv() call. Next time, return it from the buffer instead of attaining lock.
    buffer: VecDeque<T>,
}

impl<T> Drop for Reciever<T> {
    fn drop(&mut self) {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.is_channel_still_active = false;
    }
}

impl<T> Iterator for Reciever<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.recv()
    }
}

impl<T> Reciever<T> {
    fn recv(&mut self) -> Option<T> {
        if let Some(val) = self.buffer.pop_front() {
            println!("Cached");
            return Some(val);
        }

        let mut channel = self.shared.inner.lock().unwrap();
        loop {
            match channel.queue.pop_front() {
                Some(val) => {
                    if !channel.queue.is_empty() {
                        std::mem::swap(&mut channel.queue, &mut self.buffer);
                    }
                    return Some(val);
                }
                // If all senders went out of scope, the last sender drop would have called notify on receiver which then wakes up and comes to this match below
                None if channel.senders_count == 0 => return None,
                None => {
                    // This section has to be in loop, because OS  will ensure that this thread only wakes up when other thread notifies this. But it can happen that this thread was notified due to some other reason. In that case we still don't have data, so loop happens and again this thread goes to slepp
                    channel = self.shared.available.wait(channel).unwrap(); // wait method takes in the mutex guard so that this thread is not holding the lock anymore and the sender thread can hold lock and notify this thread once it has sent data
                }
            }
        }
    }
}

// Channel
struct Shared<T> {
    inner: Mutex<Inner<T>>,
    available: Condvar, // This has to be outside of Mutex, because thread1 holding the mutex has to notify thread2 that data is available. if this is inside mutex, then thread2 will indeed be notified but sees that lock is still holded by thread1 and goes to sleep again. The implementation works without this CondVar too. But by using this, the reciever thread doesn't always be executing the loop even though there's no data in queue. This makes sure the receiver thread goes to sleep until the sender thread notifies so that CPU time is not wasted by reciever thread.
}

struct Inner<T> {
    queue: VecDeque<T>,
    senders_count: usize,
    is_channel_still_active: bool,
}

fn channel<T>() -> (Sender<T>, Reciever<T>) {
    let inner = Inner {
        queue: VecDeque::new(),
        senders_count: 1,
        is_channel_still_active: true,
    };

    let shared = Arc::new(Shared {
        inner: Mutex::new(inner),
        available: Condvar::new(),
    });

    let sender = Sender {
        shared: shared.clone(),
    };

    let receiver = Reciever {
        shared: shared.clone(),
        buffer: VecDeque::new(),
    };

    (sender, receiver)
}

fn main() {
    // Test case : 1 (Multiple senders)
    // let (mut tx, mut rx) = channel();
    // tx.send(5).ok();

    // let mut tx2 = tx.clone();
    // tx2.send(10).ok();

    // let val = rx.recv();
    // println!("{val:?}");

    // let val = rx.recv();
    // println!("{val:?}");

    //////////////////////////////////////////////////////

    // Test case : 2 (All senders goes out of scope)

    // let (mut tx, mut rx) = channel::<i32>();

    // drop(tx);

    // let val = rx.recv();
    // println!("{val:?}");

    //////////////////////////////////////////////////////

    // Test case : 3 (Reciever goes out of scope)

    // let (mut tx, mut rx) = channel::<i32>();

    // drop(rx);

    // let val = tx.send(5);
    // println!("{val:?}");

    //////////////////////////////////////////////////////

    // Test case : 4 (Iterator and Buffer cache)

    // let (mut tx, mut rx) = channel();
    // tx.send(5).ok();
    // tx.send(10).ok();
    // tx.send(15).ok();

    // println!("{:?}", rx.next()); // This one attains lock and fills the element 10 and 15 to buffer and returns 5
    // println!("{:?}", rx.next()); // This one just returns 10 from buffer without attaining lock
    // println!("{:?}", rx.next()); // This one just returns 15 from buffer without attaining lock

}
