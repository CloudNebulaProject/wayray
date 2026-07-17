//! Event-ports poller for illumos.
//!
//! illumos' native readiness mechanism is event ports (`port_create(3C)`);
//! `epoll(5)` exists there only as a Linux-compatibility shim and should not
//! be used by native code. rustix exposes event ports on solarish targets
//! (and wayland-backend already enables its `event` feature), so the server
//! poller uses them directly.
//!
//! Event-port associations are one-shot: delivering an event removes the
//! association, so the dispatch loop re-associates ready fds after handling
//! them (see `dispatch_all_clients` in `common_poll.rs`).

use std::io;
use std::os::unix::io::{AsFd, OwnedFd};

use rustix::event::port;
use rustix::event::{PollFlags, Timespec};

/// Create a new event port.
pub(crate) fn create() -> io::Result<OwnedFd> {
    Ok(port::create()?)
}

/// Associate `fd` for read readiness, tagged with `data`. Also used to re-arm
/// an association consumed by event delivery.
pub(crate) fn add(port_fd: &impl AsFd, fd: &impl AsFd, data: u64) -> io::Result<()> {
    // SAFETY: the fd is owned by the client map, which keeps it alive for as
    // long as it is registered; the userdata pointer is a plain integer tag
    // and is never dereferenced.
    unsafe {
        port::associate_fd(port_fd, fd.as_fd(), PollFlags::IN, data as usize as *mut _)?;
    }
    Ok(())
}

/// Remove `fd`'s association, if any. Because associations are one-shot, the
/// association may already have been consumed by event delivery; "not
/// associated" (`ENOENT`) is therefore treated as success.
pub(crate) fn delete(port_fd: &impl AsFd, fd: &impl AsFd) -> io::Result<()> {
    // SAFETY: dissociating an fd does not touch its resources.
    match unsafe { port::dissociate_fd(port_fd, fd.as_fd()) } {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Non-blocking wait: returns the tags of all ready associations (empty when
/// nothing is ready).
pub(crate) fn wait_nonblocking(
    port_fd: &impl AsFd,
    max_events: usize,
) -> io::Result<Vec<u64>> {
    let mut events: Vec<port::Event> = Vec::with_capacity(max_events.max(1));
    match port::getn(
        port_fd,
        rustix::buffer::spare_capacity(&mut events),
        1,
        Some(&Timespec::default()),
    ) {
        Ok(_) => {}
        // A zero timeout with nothing pending reports ETIME.
        Err(rustix::io::Errno::TIME) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    }
    Ok(events.iter().map(|e| e.userdata() as usize as u64).collect())
}
