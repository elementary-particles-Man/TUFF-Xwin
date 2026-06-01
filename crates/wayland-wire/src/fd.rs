use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

#[derive(Debug)]
pub struct WireOwnedFd(pub OwnedFd);

impl WireOwnedFd {
    pub fn from_raw(fd: RawFd) -> Self {
        Self(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

pub fn send_with_fds(stream: &UnixStream, data: &[u8], fds: &[RawFd]) -> std::io::Result<usize> {
    use libc::{
        c_void, iovec, msghdr, sendmsg, CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_SPACE, SCM_RIGHTS,
        SOL_SOCKET,
    };
    use std::ptr;

    let mut msg: msghdr = unsafe { std::mem::zeroed() };
    let mut io = iovec { iov_base: data.as_ptr() as *mut c_void, iov_len: data.len() };

    msg.msg_iov = &mut io;
    msg.msg_iovlen = 1;

    let mut control_buf = [0u8; 128]; // Enough for a few FDs
    if !fds.is_empty() {
        msg.msg_control = control_buf.as_mut_ptr() as *mut c_void;
        msg.msg_controllen =
            unsafe { CMSG_SPACE((fds.len() * std::mem::size_of::<RawFd>()) as u32) } as _;

        let cmsg = unsafe { CMSG_FIRSTHDR(&msg) };
        if !cmsg.is_null() {
            unsafe {
                (*cmsg).cmsg_level = SOL_SOCKET;
                (*cmsg).cmsg_type = SCM_RIGHTS;
                (*cmsg).cmsg_len = CMSG_LEN((fds.len() * std::mem::size_of::<RawFd>()) as u32) as _;
                let data_ptr = CMSG_DATA(cmsg);
                ptr::copy_nonoverlapping(
                    fds.as_ptr() as *const u8,
                    data_ptr,
                    fds.len() * std::mem::size_of::<RawFd>(),
                );
            }
        }
    }

    let n = unsafe { sendmsg(stream.as_raw_fd(), &msg, 0) };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn recv_with_fds(
    stream: &UnixStream,
    buf: &mut [u8],
) -> std::io::Result<(usize, Vec<WireOwnedFd>)> {
    use libc::{
        c_void, iovec, msghdr, recvmsg, CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR,
        SCM_RIGHTS, SOL_SOCKET,
    };
    use std::mem;

    let mut msg: msghdr = unsafe { mem::zeroed() };
    let mut io = iovec { iov_base: buf.as_mut_ptr() as *mut c_void, iov_len: buf.len() };

    msg.msg_iov = &mut io;
    msg.msg_iovlen = 1;

    let mut control_buf = [0u8; 128];
    msg.msg_control = control_buf.as_mut_ptr() as *mut c_void;
    msg.msg_controllen = control_buf.len() as _;

    let n = unsafe { recvmsg(stream.as_raw_fd(), &mut msg, 0) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut received_fds = Vec::new();
    let mut cmsg = unsafe { CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        if unsafe { (*cmsg).cmsg_level == SOL_SOCKET && (*cmsg).cmsg_type == SCM_RIGHTS } {
            let data_ptr = unsafe { CMSG_DATA(cmsg) };
            let len = unsafe { (*cmsg).cmsg_len } as usize - unsafe { CMSG_LEN(0) } as usize;
            let count = len / mem::size_of::<RawFd>();

            unsafe {
                let fds_ptr = data_ptr as *const RawFd;
                for i in 0..count {
                    let fd = fds_ptr.add(i).read();
                    received_fds.push(WireOwnedFd::from_raw(fd));
                }
            }
        }
        cmsg = unsafe { CMSG_NXTHDR(&msg, cmsg) };
    }

    Ok((n as usize, received_fds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};
    use tempfile::tempfile;

    #[test]
    fn test_scm_rights_send_recv() {
        let (s1, s2) = UnixStream::pair().unwrap();

        let mut temp = tempfile().unwrap();
        temp.write_all(b"hello fd").unwrap();
        temp.flush().unwrap();

        let fd = temp.as_raw_fd();

        let send_handle = std::thread::spawn(move || {
            send_with_fds(&s1, b"header", &[fd]).expect("send failed");
        });

        let mut buf = [0u8; 10];
        let (n, mut received) = recv_with_fds(&s2, &mut buf).expect("recv failed");
        assert_eq!(&buf[..n], b"header");
        assert_eq!(received.len(), 1);

        let mut received_file = std::fs::File::from(received.remove(0).0);
        let mut content = String::new();
        received_file.seek(std::io::SeekFrom::Start(0)).unwrap();
        received_file.read_to_string(&mut content).expect("read failed");
        assert_eq!(content, "hello fd");

        send_handle.join().unwrap();
    }
}
