//! The runtime's I/O ports: the datagram transport and the clock.
//!
//! `ServerRuntime` reads and writes datagrams only through `Transport` and
//! reads the time on its tick path only through `Clock`, so a test can drive
//! a whole runtime without a socket and with time it advances by hand. The
//! process uses `UdpTransport` and `SystemClock`; tests use
//! `MemoryTransport` and `ManualClock` through `ServerRuntime::for_test`.
#[cfg(test)]
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    time::Instant,
};

/// Non-blocking datagram I/O. `recv` returns `WouldBlock` when nothing is
/// queued, which ends the dispatcher's receive loop for the tick.
pub(crate) trait Transport {
    fn recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    fn send_to(&self, bytes: &[u8], addr: SocketAddr) -> io::Result<usize>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
    /// Like `recv` without consuming the datagram; test fixtures use it to
    /// wait for kernel delivery before running the receive loop.
    #[cfg(test)]
    fn peek(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
}

/// Monotonic time as the tick path sees it.
pub(crate) trait Clock {
    fn now(&self) -> Instant;
}

/// A non-blocking `UdpSocket`.
pub(crate) struct UdpTransport(pub(crate) UdpSocket);
impl Transport for UdpTransport {
    fn recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.0.recv_from(buf)
    }
    fn send_to(&self, bytes: &[u8], addr: SocketAddr) -> io::Result<usize> {
        self.0.send_to(bytes, addr)
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
    #[cfg(test)]
    fn peek(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.0.peek_from(buf)
    }
}

pub(crate) struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Datagram queues in memory. The handle is shared: a test keeps a clone to
/// push inbound datagrams and to read what the runtime sent.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct MemoryTransport {
    local: SocketAddr,
    inbound: Arc<Mutex<VecDeque<(SocketAddr, Vec<u8>)>>>,
    outbound: Arc<Mutex<Vec<(SocketAddr, Vec<u8>)>>>,
}
#[cfg(test)]
impl MemoryTransport {
    pub(crate) fn new(local: SocketAddr) -> Self {
        Self {
            local,
            inbound: Arc::new(Mutex::new(VecDeque::new())),
            outbound: Arc::new(Mutex::new(Vec::new())),
        }
    }
    /// Queues one datagram from `from` for the next receive loop.
    pub(crate) fn push_inbound(&self, from: SocketAddr, bytes: Vec<u8>) {
        self.inbound.lock().unwrap().push_back((from, bytes));
    }
    /// Everything the runtime sent since the last call, in send order.
    pub(crate) fn take_outbound(&self) -> Vec<(SocketAddr, Vec<u8>)> {
        std::mem::take(&mut *self.outbound.lock().unwrap())
    }
}
#[cfg(test)]
impl Transport for MemoryTransport {
    fn recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let Some((from, bytes)) = self.inbound.lock().unwrap().pop_front() else {
            return Err(io::ErrorKind::WouldBlock.into());
        };
        let len = bytes.len().min(buf.len());
        buf[..len].copy_from_slice(&bytes[..len]);
        Ok((len, from))
    }
    fn send_to(&self, bytes: &[u8], addr: SocketAddr) -> io::Result<usize> {
        self.outbound.lock().unwrap().push((addr, bytes.to_vec()));
        Ok(bytes.len())
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local)
    }
    fn peek(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let queue = self.inbound.lock().unwrap();
        let Some((from, bytes)) = queue.front() else {
            return Err(io::ErrorKind::WouldBlock.into());
        };
        let len = bytes.len().min(buf.len());
        buf[..len].copy_from_slice(&bytes[..len]);
        Ok((len, *from))
    }
}

/// Time that moves only when a test says so; clones share the same instant.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ManualClock(Arc<Mutex<Instant>>);
#[cfg(test)]
impl ManualClock {
    pub(crate) fn new(start: Instant) -> Self {
        Self(Arc::new(Mutex::new(start)))
    }
    pub(crate) fn advance(&self, by: Duration) -> Instant {
        let mut now = self.0.lock().unwrap();
        *now += by;
        *now
    }
}
#[cfg(test)]
impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}
