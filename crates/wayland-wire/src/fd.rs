use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

const MAX_ANCILLARY_FDS: usize = 16;

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

    let mut control_buf = [0u8; 128]; // Bounded ancillary-data budget.
    if fds.len() > MAX_ANCILLARY_FDS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "too many FDs for one Wayland message",
        ));
    }
    if !fds.is_empty() {
        let required_control = unsafe { CMSG_SPACE(std::mem::size_of_val(fds) as u32) } as usize;
        if required_control > control_buf.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "too many FDs for bounded Wayland ancillary buffer",
            ));
        }
        msg.msg_control = control_buf.as_mut_ptr() as *mut c_void;
        msg.msg_controllen = required_control as _;

        let cmsg = unsafe { CMSG_FIRSTHDR(&msg) };
        if !cmsg.is_null() {
            unsafe {
                (*cmsg).cmsg_level = SOL_SOCKET;
                (*cmsg).cmsg_type = SCM_RIGHTS;
                (*cmsg).cmsg_len = CMSG_LEN(std::mem::size_of_val(fds) as u32) as _;
                let data_ptr = CMSG_DATA(cmsg);
                ptr::copy_nonoverlapping(
                    fds.as_ptr() as *const u8,
                    data_ptr,
                    std::mem::size_of_val(fds),
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
        MSG_CTRUNC, SCM_RIGHTS, SOL_SOCKET,
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
    let ancillary_truncated = msg.msg_flags & MSG_CTRUNC != 0;

    let mut received_fds = Vec::new();
    let mut cmsg = unsafe { CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        if unsafe { (*cmsg).cmsg_level == SOL_SOCKET && (*cmsg).cmsg_type == SCM_RIGHTS } {
            let data_ptr = unsafe { CMSG_DATA(cmsg) };
            let header_len = unsafe { CMSG_LEN(0) } as usize;
            let cmsg_len = unsafe { (*cmsg).cmsg_len } as usize;
            if cmsg_len < header_len {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "malformed Wayland ancillary FD header",
                ));
            }
            let len = cmsg_len - header_len;
            if !len.is_multiple_of(mem::size_of::<RawFd>()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "misaligned Wayland ancillary FD payload",
                ));
            }
            let count = len / mem::size_of::<RawFd>();
            // Take ownership of every received descriptor before returning an
            // error so malformed ancillary input cannot leak descriptors into
            // the process. Dropping this temporary vector closes them.
            let mut current = Vec::with_capacity(count.min(MAX_ANCILLARY_FDS + 1));
            unsafe {
                let fds_ptr = data_ptr as *const RawFd;
                for i in 0..count {
                    current.push(WireOwnedFd::from_raw(fds_ptr.add(i).read()));
                }
            }
            if count > MAX_ANCILLARY_FDS
                || received_fds.len().saturating_add(count) > MAX_ANCILLARY_FDS
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "too many FDs in one Wayland ancillary message",
                ));
            }
            received_fds.extend(current);
        }
        cmsg = unsafe { CMSG_NXTHDR(&msg, cmsg) };
    }

    if ancillary_truncated {
        // received_fds owns every descriptor that fit in the bounded control
        // buffer; dropping it on this error closes those descriptors.
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "truncated Wayland ancillary FD data",
        ));
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

    #[test]
    fn rejects_ancillary_fd_count_before_sendmsg() {
        let (s1, _s2) = UnixStream::pair().unwrap();
        let fds = [0; MAX_ANCILLARY_FDS + 1];
        let error = send_with_fds(&s1, b"x", &fds).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
