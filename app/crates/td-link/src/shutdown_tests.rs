use super::*;
use std::io::{self, Read, Write};
use std::sync::{Arc, mpsc};
use std::thread;

struct Stalled {
    flush: bool,
    release: mpsc::Receiver<()>,
    dropped: mpsc::Sender<()>,
}
impl Write for Stalled {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.flush {
            Ok(bytes.len())
        } else {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        let _ = self.release.recv();
        Ok(())
    }
}
impl Drop for Stalled {
    fn drop(&mut self) {
        let _ = self.dropped.send(());
    }
}
struct Reader;
impl Read for Reader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        thread::sleep(crate::READ_TICK);
        Err(io::ErrorKind::TimedOut.into())
    }
}
impl crate::Transport for Stalled {
    fn split(self: Box<Self>) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        Ok((Box::new(Reader), self))
    }
}
fn closes(flush: bool) {
    let (release, blocked) = mpsc::channel();
    let (dropped, closed) = mpsc::channel();
    let (session, _) = Session::open(
        Box::new(Stalled {
            flush,
            release: blocked,
            dropped,
        }),
        SessionConfig::default(),
    )
    .unwrap();
    let session = Arc::new(session);
    let (done, finished) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = session.request(&td_core::Request::Hello, Duration::from_millis(100));
        session.close();
        done.send(result).unwrap();
    });
    let timely = finished.recv_timeout(Duration::from_secs(2));
    // Always unblock a regressed drain before asserting, so the test leaves no stuck thread.
    let _ = release.send(());
    worker.join().unwrap();
    assert!(
        timely.is_ok(),
        "stalled serial transmission prevented link close"
    );
    if !flush {
        assert_eq!(
            timely.unwrap(),
            Err(LinkError::Timeout(Duration::from_millis(100)))
        );
    }
    closed.recv_timeout(Duration::from_secs(1)).unwrap();
}
#[test]
fn stalled_serial_flush_cannot_prevent_link_close() {
    closes(true);
}
#[test]
fn stalled_serial_write_cannot_prevent_link_close() {
    closes(false);
}
